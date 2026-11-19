//! # Background job queue
//!
//! Durable-style job processing with the features a production queue needs:
//!
//! * **Priority queues** – `High`, `Normal`, `Low`; FIFO inside a priority.
//! * **Delayed and scheduled jobs** – `run_at` for one-offs, cron expressions
//!   (`m h dom mon dow`, UTC) for recurring work.
//! * **Retries with backoff** – fixed or exponential; a job that runs out of
//!   attempts moves to the dead-letter list and raises an alert.
//! * **Concurrency control and rate limiting** per queue.
//! * **Progress tracking** and **metrics** (counts, throughput, average run
//!   time, oldest wait) plus a [`QueueHealth`] verdict for the dashboard.
//! * **Manual retry** of dead jobs ([`JobQueue::retry_dead`]) and job history.
//!
//! The queue is a state machine over caller-supplied timestamps (epoch seconds),
//! which keeps it deterministic and testable; workers call [`JobQueue::claim`],
//! do the work, then [`JobQueue::complete`] or [`JobQueue::fail`].
//! [`JobQueue::work_once`] wraps that loop for a closure handler.
//!
//! State is held in memory; a Redis or SQL store plugs in by persisting the `Job`
//! records this module produces. See `docs/JOB_QUEUE.md`.

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;
/// Jobs waiting longer than this make a queue `Degraded`.
pub const DEFAULT_STALE_WAIT_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    High,
    Normal,
    Low,
}

impl Priority {
    const ORDER: [Priority; 3] = [Priority::High, Priority::Normal, Priority::Low];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backoff {
    Fixed { secs: u64 },
    Exponential { base_secs: u64, max_secs: u64 },
}

impl Backoff {
    /// Delay before retry number `attempt` (1 = after the first failure).
    pub fn delay(self, attempt: u32) -> u64 {
        match self {
            Backoff::Fixed { secs } => secs,
            Backoff::Exponential { base_secs, max_secs } => {
                let shift = attempt.saturating_sub(1).min(32);
                base_secs.saturating_mul(1u64 << shift).min(max_secs)
            }
        }
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff::Exponential {
            base_secs: 5,
            max_secs: 3600,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Waiting,
    Delayed,
    Active,
    Completed,
    /// Out of attempts: sits in the dead-letter list until retried by hand.
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub queue: String,
    pub name: String,
    pub payload: Value,
    pub priority: Priority,
    /// Do not start before this time (epoch seconds).
    pub run_at: Option<u64>,
    pub max_attempts: u32,
    pub backoff: Backoff,
}

impl JobSpec {
    pub fn new(queue: &str, name: &str, payload: Value) -> Self {
        JobSpec {
            queue: queue.to_string(),
            name: name.to_string(),
            payload,
            priority: Priority::Normal,
            run_at: None,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            backoff: Backoff::default(),
        }
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
    pub fn run_at(mut self, at: u64) -> Self {
        self.run_at = Some(at);
        self
    }
    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub spec: JobSpec,
    pub state: JobState,
    pub attempts: u32,
    pub progress: u8,
    pub progress_message: String,
    pub result: Option<Value>,
    pub last_error: Option<String>,
    pub created_at: u64,
    pub available_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub max_starts: u32,
    pub per_secs: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct QueueConfig {
    /// Jobs of this queue allowed to run at once.
    pub concurrency: usize,
    pub rate_limit: Option<RateLimit>,
    pub stale_wait_secs: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig {
            concurrency: 4,
            rate_limit: None,
            stale_wait_secs: DEFAULT_STALE_WAIT_SECS,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueueError {
    #[error("job {0} not found")]
    NotFound(u64),
    #[error("job {0} is not {1}")]
    WrongState(u64, &'static str),
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),
    #[error("queue state unavailable")]
    Poisoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueHealth {
    Healthy,
    /// Dead jobs exist, or work has been waiting too long.
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueueMetrics {
    pub queue: String,
    pub waiting: usize,
    pub delayed: usize,
    pub active: usize,
    pub completed: usize,
    pub dead: usize,
    /// Jobs completed in the last 60 seconds.
    pub throughput_per_min: usize,
    pub avg_duration_secs: Option<f64>,
    pub oldest_wait_secs: Option<u64>,
    pub health: QueueHealth,
}

// ---------------------------------------------------------------------------
// Cron
// ---------------------------------------------------------------------------

/// A five-field cron schedule evaluated in UTC. Each field accepts `*`, `n`,
/// `a-b`, comma lists, and `/step` (`*/5`, `10-30/10`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    weekdays: Vec<u32>,
    day_restricted: bool,
    weekday_restricted: bool,
}

fn parse_field(text: &str, min: u32, max: u32) -> Result<Vec<u32>, QueueError> {
    let bad = || QueueError::InvalidCron(text.to_string());
    let mut values = Vec::new();
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().map_err(|_| bad())?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(bad());
        }
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?)
        } else {
            let n: u32 = range.parse().map_err(|_| bad())?;
            // `5/10` means "from 5 every 10"; a bare number is just itself.
            (n, if part.contains('/') { max } else { n })
        };
        if start < min || end > max || start > end {
            return Err(bad());
        }
        values.extend((start..=end).step_by(step as usize));
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

impl Cron {
    pub fn parse(expression: &str) -> Result<Cron, QueueError> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(QueueError::InvalidCron(expression.to_string()));
        }
        Ok(Cron {
            minutes: parse_field(fields[0], 0, 59)?,
            hours: parse_field(fields[1], 0, 23)?,
            days: parse_field(fields[2], 1, 31)?,
            months: parse_field(fields[3], 1, 12)?,
            weekdays: parse_field(fields[4], 0, 6)?,
            day_restricted: fields[2] != "*",
            weekday_restricted: fields[4] != "*",
        })
    }

    fn matches(&self, at: DateTime<Utc>) -> bool {
        let day_ok = self.days.contains(&at.day());
        let weekday_ok = self.weekdays.contains(&at.weekday().num_days_from_sunday());
        // Standard cron: when both day fields are restricted, either may match.
        let date_ok = match (self.day_restricted, self.weekday_restricted) {
            (true, true) => day_ok || weekday_ok,
            (true, false) => day_ok,
            (false, true) => weekday_ok,
            (false, false) => true,
        };
        self.minutes.contains(&at.minute())
            && self.hours.contains(&at.hour())
            && self.months.contains(&at.month())
            && date_ok
    }

    /// First matching minute strictly after `after` (epoch seconds).
    pub fn next_after(&self, after: u64) -> Option<u64> {
        let start = (after / 60 + 1) * 60;
        // Four years covers a Feb 29 schedule.
        for step in 0..(4 * 366 * 24 * 60u64) {
            let ts = start + step * 60;
            let at = DateTime::<Utc>::from_timestamp(ts as i64, 0)?;
            if self.matches(at) {
                return Some(ts);
            }
        }
        None
    }
}

struct CronEntry {
    name: String,
    cron: Cron,
    template: JobSpec,
    next_run: u64,
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

type AlertHook = Arc<dyn Fn(&Job) + Send + Sync>;

#[derive(Default)]
struct Inner {
    next_id: u64,
    jobs: BTreeMap<u64, Job>,
    configs: HashMap<String, QueueConfig>,
    /// Recent start times per queue, for rate limiting.
    starts: HashMap<String, VecDeque<u64>>,
    crons: Vec<CronEntry>,
}

pub struct JobQueue {
    inner: Mutex<Inner>,
    on_dead: Option<AlertHook>,
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue {
    pub fn new() -> Self {
        JobQueue {
            inner: Mutex::new(Inner::default()),
            on_dead: None,
        }
    }

    /// Called whenever a job lands in the dead-letter list (page someone).
    pub fn with_dead_letter_alert(mut self, hook: impl Fn(&Job) + Send + Sync + 'static) -> Self {
        self.on_dead = Some(Arc::new(hook));
        self
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, QueueError> {
        self.inner.lock().map_err(|_| QueueError::Poisoned)
    }

    pub fn configure(&self, queue: &str, config: QueueConfig) -> Result<(), QueueError> {
        self.lock()?.configs.insert(queue.to_string(), config);
        Ok(())
    }

    pub fn enqueue(&self, spec: JobSpec, now: u64) -> Result<u64, QueueError> {
        let mut inner = self.lock()?;
        Ok(Self::insert(&mut inner, spec, now))
    }

    fn insert(inner: &mut Inner, spec: JobSpec, now: u64) -> u64 {
        inner.next_id += 1;
        let id = inner.next_id;
        let available_at = spec.run_at.unwrap_or(now);
        let state = if available_at > now { JobState::Delayed } else { JobState::Waiting };
        inner.jobs.insert(
            id,
            Job {
                id,
                spec,
                state,
                attempts: 0,
                progress: 0,
                progress_message: "Queued".to_string(),
                result: None,
                last_error: None,
                created_at: now,
                available_at,
                started_at: None,
                finished_at: None,
            },
        );
        id
    }

    /// Register a recurring job. Each time the schedule fires, a copy of
    /// `template` is enqueued.
    pub fn schedule_cron(&self, name: &str, expression: &str, template: JobSpec, now: u64) -> Result<(), QueueError> {
        let cron = Cron::parse(expression)?;
        let next_run = cron
            .next_after(now)
            .ok_or_else(|| QueueError::InvalidCron(expression.to_string()))?;
        self.lock()?.crons.push(CronEntry {
            name: name.to_string(),
            cron,
            template,
            next_run,
        });
        Ok(())
    }

    /// Enqueue every cron job that is due. Returns the new job ids. Missed
    /// firings (the process was down) collapse into one run.
    pub fn tick(&self, now: u64) -> Result<Vec<u64>, QueueError> {
        let mut inner = self.lock()?;
        let mut due = Vec::new();
        for entry in inner.crons.iter_mut() {
            if entry.next_run <= now {
                due.push(entry.template.clone());
                entry.next_run = entry.cron.next_after(now).unwrap_or(u64::MAX);
            }
        }
        Ok(due
            .into_iter()
            .map(|spec| Self::insert(&mut inner, spec, now))
            .collect())
    }

    /// Next run time of a named cron job.
    pub fn next_cron_run(&self, name: &str) -> Result<Option<u64>, QueueError> {
        Ok(self.lock()?.crons.iter().find(|c| c.name == name).map(|c| c.next_run))
    }

    /// Take the next runnable job of `queue`: highest priority first, oldest
    /// first, never exceeding the queue's concurrency or rate limit.
    pub fn claim(&self, queue: &str, now: u64) -> Result<Option<Job>, QueueError> {
        let mut inner = self.lock()?;
        let config = inner.configs.get(queue).copied().unwrap_or_default();

        // Promote delayed jobs that are due.
        for job in inner.jobs.values_mut() {
            if job.spec.queue == queue && job.state == JobState::Delayed && job.available_at <= now {
                job.state = JobState::Waiting;
            }
        }

        let active = inner
            .jobs
            .values()
            .filter(|j| j.spec.queue == queue && j.state == JobState::Active)
            .count();
        if active >= config.concurrency {
            return Ok(None);
        }

        if let Some(limit) = config.rate_limit {
            let starts = inner.starts.entry(queue.to_string()).or_default();
            while starts.front().is_some_and(|t| now.saturating_sub(*t) >= limit.per_secs) {
                starts.pop_front();
            }
            if starts.len() >= limit.max_starts as usize {
                return Ok(None);
            }
        }

        let next = Priority::ORDER.iter().find_map(|priority| {
            inner
                .jobs
                .values()
                .filter(|j| {
                    j.spec.queue == queue && j.state == JobState::Waiting && j.spec.priority == *priority
                })
                .min_by_key(|j| (j.available_at, j.id))
                .map(|j| j.id)
        });
        let Some(id) = next else { return Ok(None) };

        inner.starts.entry(queue.to_string()).or_default().push_back(now);
        let job = inner.jobs.get_mut(&id).ok_or(QueueError::NotFound(id))?;
        job.state = JobState::Active;
        job.attempts += 1;
        job.started_at = Some(now);
        job.progress_message = "Running".to_string();
        Ok(Some(job.clone()))
    }

    fn active_job(inner: &mut Inner, id: u64) -> Result<&mut Job, QueueError> {
        let job = inner.jobs.get_mut(&id).ok_or(QueueError::NotFound(id))?;
        if job.state != JobState::Active {
            return Err(QueueError::WrongState(id, "active"));
        }
        Ok(job)
    }

    pub fn report_progress(&self, id: u64, percent: u8, message: &str) -> Result<(), QueueError> {
        let mut inner = self.lock()?;
        let job = Self::active_job(&mut inner, id)?;
        job.progress = percent.min(100);
        job.progress_message = message.to_string();
        Ok(())
    }

    pub fn complete(&self, id: u64, result: Value, now: u64) -> Result<(), QueueError> {
        let mut inner = self.lock()?;
        let job = Self::active_job(&mut inner, id)?;
        job.state = JobState::Completed;
        job.progress = 100;
        job.progress_message = "Completed".to_string();
        job.result = Some(result);
        job.finished_at = Some(now);
        Ok(())
    }

    /// Record a failure: retry after the backoff, or dead-letter the job when
    /// attempts are exhausted.
    pub fn fail(&self, id: u64, error: &str, now: u64) -> Result<JobState, QueueError> {
        let dead = {
            let mut inner = self.lock()?;
            let job = Self::active_job(&mut inner, id)?;
            job.last_error = Some(error.to_string());
            if job.attempts >= job.spec.max_attempts {
                job.state = JobState::Dead;
                job.finished_at = Some(now);
                job.progress_message = "Dead".to_string();
                Some(job.clone())
            } else {
                job.available_at = now + job.spec.backoff.delay(job.attempts);
                job.state = JobState::Delayed;
                job.progress_message = format!("Retrying (attempt {} failed)", job.attempts);
                None
            }
        };
        match dead {
            Some(job) => {
                if let Some(hook) = &self.on_dead {
                    hook(&job);
                }
                Ok(JobState::Dead)
            }
            None => Ok(JobState::Delayed),
        }
    }

    /// Manual retry from the dashboard: a dead job gets a fresh set of attempts.
    pub fn retry_dead(&self, id: u64, now: u64) -> Result<(), QueueError> {
        let mut inner = self.lock()?;
        let job = inner.jobs.get_mut(&id).ok_or(QueueError::NotFound(id))?;
        if job.state != JobState::Dead {
            return Err(QueueError::WrongState(id, "dead"));
        }
        job.state = JobState::Waiting;
        job.attempts = 0;
        job.available_at = now;
        job.finished_at = None;
        job.progress = 0;
        job.progress_message = "Queued (manual retry)".to_string();
        Ok(())
    }

    /// Claim and run one job with `handler`. `Ok(false)` means nothing to do.
    pub fn work_once(
        &self,
        queue: &str,
        now: u64,
        handler: impl FnOnce(&Job) -> Result<Value, String>,
    ) -> Result<bool, QueueError> {
        let Some(job) = self.claim(queue, now)? else {
            return Ok(false);
        };
        match handler(&job) {
            Ok(result) => self.complete(job.id, result, now)?,
            Err(error) => {
                self.fail(job.id, &error, now)?;
            }
        }
        Ok(true)
    }

    // -- dashboard --------------------------------------------------------

    pub fn job(&self, id: u64) -> Result<Option<Job>, QueueError> {
        Ok(self.lock()?.jobs.get(&id).cloned())
    }

    /// Dead-letter list for a queue.
    pub fn dead_letters(&self, queue: &str) -> Result<Vec<Job>, QueueError> {
        Ok(self
            .lock()?
            .jobs
            .values()
            .filter(|j| j.spec.queue == queue && j.state == JobState::Dead)
            .cloned()
            .collect())
    }

    /// Most recently finished jobs (completed or dead), newest first.
    pub fn history(&self, queue: &str, limit: usize) -> Result<Vec<Job>, QueueError> {
        let mut finished: Vec<Job> = self
            .lock()?
            .jobs
            .values()
            .filter(|j| j.spec.queue == queue && matches!(j.state, JobState::Completed | JobState::Dead))
            .cloned()
            .collect();
        finished.sort_by_key(|j| std::cmp::Reverse((j.finished_at, j.id)));
        finished.truncate(limit);
        Ok(finished)
    }

    pub fn metrics(&self, queue: &str, now: u64) -> Result<QueueMetrics, QueueError> {
        let inner = self.lock()?;
        let config = inner.configs.get(queue).copied().unwrap_or_default();
        let jobs: Vec<&Job> = inner.jobs.values().filter(|j| j.spec.queue == queue).collect();
        let count = |state: JobState| jobs.iter().filter(|j| j.state == state).count();

        let durations: Vec<u64> = jobs
            .iter()
            .filter(|j| j.state == JobState::Completed)
            .filter_map(|j| Some(j.finished_at?.saturating_sub(j.started_at?)))
            .collect();
        let avg_duration_secs = (!durations.is_empty())
            .then(|| durations.iter().sum::<u64>() as f64 / durations.len() as f64);

        let oldest_wait_secs = jobs
            .iter()
            .filter(|j| j.state == JobState::Waiting)
            .map(|j| now.saturating_sub(j.available_at))
            .max();

        let dead = count(JobState::Dead);
        let stale = oldest_wait_secs.is_some_and(|wait| wait > config.stale_wait_secs);

        Ok(QueueMetrics {
            queue: queue.to_string(),
            waiting: count(JobState::Waiting),
            delayed: count(JobState::Delayed),
            active: count(JobState::Active),
            completed: count(JobState::Completed),
            dead,
            throughput_per_min: jobs
                .iter()
                .filter(|j| j.state == JobState::Completed && j.finished_at.is_some_and(|t| t + 60 > now))
                .count(),
            avg_duration_secs,
            oldest_wait_secs,
            health: if dead > 0 || stale { QueueHealth::Degraded } else { QueueHealth::Healthy },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn spec(name: &str) -> JobSpec {
        JobSpec::new("work", name, json!({"n": name}))
    }

    fn names(queue: &JobQueue, count: usize, now: u64) -> Vec<String> {
        // Concurrency is raised so every claim is allowed.
        (0..count)
            .filter_map(|_| queue.claim("work", now).unwrap())
            .map(|job| job.spec.name)
            .collect()
    }

    fn roomy() -> JobQueue {
        let queue = JobQueue::new();
        queue
            .configure("work", QueueConfig { concurrency: 100, ..QueueConfig::default() })
            .unwrap();
        queue
    }

    #[test]
    fn jobs_run_by_priority_then_in_arrival_order() {
        let queue = roomy();
        queue.enqueue(spec("low-1").priority(Priority::Low), 10).unwrap();
        queue.enqueue(spec("normal-1"), 11).unwrap();
        queue.enqueue(spec("high-1").priority(Priority::High), 12).unwrap();
        queue.enqueue(spec("normal-2"), 13).unwrap();
        queue.enqueue(spec("high-2").priority(Priority::High), 14).unwrap();

        assert_eq!(names(&queue, 6, 100), ["high-1", "high-2", "normal-1", "normal-2", "low-1"]);
    }

    #[test]
    fn delayed_jobs_wait_for_their_time() {
        let queue = roomy();
        let id = queue.enqueue(spec("later").run_at(500), 100).unwrap();
        assert_eq!(queue.job(id).unwrap().unwrap().state, JobState::Delayed);

        assert!(queue.claim("work", 499).unwrap().is_none());
        assert_eq!(queue.claim("work", 500).unwrap().unwrap().id, id);
    }

    #[test]
    fn concurrency_is_capped_per_queue() {
        let queue = JobQueue::new();
        queue.configure("work", QueueConfig { concurrency: 2, ..QueueConfig::default() }).unwrap();
        for i in 0..4 {
            queue.enqueue(spec(&format!("j{i}")), 0).unwrap();
        }
        let a = queue.claim("work", 0).unwrap().unwrap();
        let _b = queue.claim("work", 0).unwrap().unwrap();
        assert!(queue.claim("work", 0).unwrap().is_none(), "third must wait");

        queue.complete(a.id, json!(null), 1).unwrap();
        assert!(queue.claim("work", 1).unwrap().is_some());
    }

    #[test]
    fn rate_limit_spaces_out_starts() {
        let queue = JobQueue::new();
        queue
            .configure(
                "work",
                QueueConfig {
                    concurrency: 100,
                    rate_limit: Some(RateLimit { max_starts: 2, per_secs: 10 }),
                    ..QueueConfig::default()
                },
            )
            .unwrap();
        for i in 0..4 {
            queue.enqueue(spec(&format!("j{i}")), 0).unwrap();
        }
        assert!(queue.claim("work", 0).unwrap().is_some());
        assert!(queue.claim("work", 1).unwrap().is_some());
        assert!(queue.claim("work", 5).unwrap().is_none(), "limit of 2 per 10s reached");
        assert!(queue.claim("work", 10).unwrap().is_some(), "first start left the window");
    }

    #[test]
    fn failures_retry_with_exponential_backoff_then_die() {
        let queue = roomy();
        let id = queue
            .enqueue(
                spec("flaky")
                    .max_attempts(3)
                    .backoff(Backoff::Exponential { base_secs: 10, max_secs: 1000 }),
                0,
            )
            .unwrap();

        queue.claim("work", 0).unwrap();
        assert_eq!(queue.fail(id, "boom", 0).unwrap(), JobState::Delayed);
        assert!(queue.claim("work", 9).unwrap().is_none());

        queue.claim("work", 10).unwrap(); // attempt 2, retried after 20s
        assert_eq!(queue.fail(id, "boom", 10).unwrap(), JobState::Delayed);
        assert!(queue.claim("work", 29).unwrap().is_none());

        queue.claim("work", 30).unwrap(); // attempt 3 = last
        assert_eq!(queue.fail(id, "boom again", 30).unwrap(), JobState::Dead);
        let job = queue.job(id).unwrap().unwrap();
        assert_eq!((job.state, job.attempts), (JobState::Dead, 3));
        assert_eq!(job.last_error.as_deref(), Some("boom again"));
    }

    #[test]
    fn backoff_delays() {
        let exp = Backoff::Exponential { base_secs: 5, max_secs: 60 };
        assert_eq!([1, 2, 3, 4, 5, 6].map(|a| exp.delay(a)), [5, 10, 20, 40, 60, 60]);
        assert_eq!(Backoff::Fixed { secs: 7 }.delay(9), 7);
        assert_eq!(exp.delay(500), 60, "no overflow on huge attempt counts");
    }

    #[test]
    fn dead_jobs_alert_and_can_be_retried_manually() {
        let alerts = Arc::new(AtomicUsize::new(0));
        let seen = alerts.clone();
        let queue = JobQueue::new().with_dead_letter_alert(move |job| {
            assert_eq!(job.state, JobState::Dead);
            seen.fetch_add(1, Ordering::SeqCst);
        });
        let id = queue.enqueue(spec("doomed").max_attempts(1), 0).unwrap();
        queue.claim("work", 0).unwrap();
        queue.fail(id, "nope", 0).unwrap();

        assert_eq!(alerts.load(Ordering::SeqCst), 1);
        assert_eq!(queue.dead_letters("work").unwrap().len(), 1);
        assert_eq!(queue.metrics("work", 1).unwrap().health, QueueHealth::Degraded);

        queue.retry_dead(id, 50).unwrap();
        let again = queue.claim("work", 50).unwrap().unwrap();
        assert_eq!((again.id, again.attempts), (id, 1));
        assert_eq!(queue.retry_dead(id, 51), Err(QueueError::WrongState(id, "dead")));
    }

    #[test]
    fn progress_is_tracked_and_clamped() {
        let queue = roomy();
        let id = queue.enqueue(spec("long"), 0).unwrap();
        queue.claim("work", 0).unwrap();

        queue.report_progress(id, 40, "halfway-ish").unwrap();
        let job = queue.job(id).unwrap().unwrap();
        assert_eq!((job.progress, job.progress_message.as_str()), (40, "halfway-ish"));
        queue.report_progress(id, 250, "over").unwrap();
        assert_eq!(queue.job(id).unwrap().unwrap().progress, 100);

        queue.complete(id, json!({"ok": true}), 5).unwrap();
        assert_eq!(queue.report_progress(id, 1, "late"), Err(QueueError::WrongState(id, "active")));
    }

    #[test]
    fn metrics_and_health() {
        let queue = roomy();
        for i in 0..3 {
            queue.enqueue(spec(&format!("j{i}")), 0).unwrap();
        }
        queue.enqueue(spec("future").run_at(10_000), 0).unwrap();
        let a = queue.claim("work", 100).unwrap().unwrap();
        queue.complete(a.id, json!(null), 104).unwrap();
        let _active = queue.claim("work", 105).unwrap().unwrap();

        let m = queue.metrics("work", 110).unwrap();
        assert_eq!((m.waiting, m.delayed, m.active, m.completed, m.dead), (1, 1, 1, 1, 0));
        assert_eq!(m.throughput_per_min, 1);
        assert_eq!(m.avg_duration_secs, Some(4.0));
        assert_eq!(m.oldest_wait_secs, Some(110));
        assert_eq!(m.health, QueueHealth::Healthy);

        let stale = queue.metrics("work", 100 + DEFAULT_STALE_WAIT_SECS + 1).unwrap();
        assert_eq!(stale.health, QueueHealth::Degraded);
        assert_eq!(queue.metrics("empty", 0).unwrap().health, QueueHealth::Healthy);
    }

    #[test]
    fn history_lists_finished_jobs_newest_first() {
        let queue = roomy();
        let ids: Vec<u64> = (0..3).map(|i| queue.enqueue(spec(&format!("j{i}")), 0).unwrap()).collect();
        for (i, id) in ids.iter().enumerate() {
            queue.claim("work", i as u64).unwrap();
            queue.complete(*id, json!(i), 10 + i as u64).unwrap();
        }
        let history = queue.history("work", 2).unwrap();
        assert_eq!(history.iter().map(|j| j.id).collect::<Vec<_>>(), vec![ids[2], ids[1]]);
    }

    #[test]
    fn work_once_runs_the_handler_and_records_the_outcome() {
        let queue = roomy();
        let ok = queue.enqueue(spec("ok"), 0).unwrap();
        let bad = queue.enqueue(spec("bad").max_attempts(1), 0).unwrap();

        assert!(queue.work_once("work", 1, |job| Ok(json!({"saw": job.spec.name.clone()}))).unwrap());
        assert!(queue.work_once("work", 1, |_| Err("failed".to_string())).unwrap());
        assert!(!queue.work_once("work", 1, |_| Ok(json!(null))).unwrap());

        assert_eq!(queue.job(ok).unwrap().unwrap().result, Some(json!({"saw": "ok"})));
        assert_eq!(queue.job(bad).unwrap().unwrap().state, JobState::Dead);
    }

    #[test]
    fn cron_expressions_parse_and_find_the_next_run() {
        // 2026-01-05 10:07:30 UTC is a Monday.
        let after = 1_767_607_650;
        let every_five = Cron::parse("*/5 * * * *").unwrap();
        assert_eq!(every_five.next_after(after), Some(after - 30 + 3 * 60)); // 10:10:00

        let daily = Cron::parse("30 2 * * *").unwrap();
        let next = daily.next_after(after).unwrap();
        assert_eq!(DateTime::<Utc>::from_timestamp(next as i64, 0).unwrap().to_rfc3339(), "2026-01-06T02:30:00+00:00");

        let weekdays = Cron::parse("0 9 * * 1-5").unwrap();
        let friday_evening = 1_767_999_000; // 2026-01-09 22:10 UTC
        let next = weekdays.next_after(friday_evening).unwrap();
        assert_eq!(DateTime::<Utc>::from_timestamp(next as i64, 0).unwrap().to_rfc3339(), "2026-01-12T09:00:00+00:00");

        for bad in ["* * * *", "61 * * * *", "* 24 * * *", "*/0 * * * *", "a b c d e", "5-1 * * * *"] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cron_jobs_are_enqueued_when_due_and_missed_runs_collapse() {
        let queue = roomy();
        let start = 1_767_607_650; // 10:07:30
        queue.schedule_cron("sweep", "*/5 * * * *", spec("sweep"), start).unwrap();
        let first = queue.next_cron_run("sweep").unwrap().unwrap();
        assert_eq!(first, start - 30 + 3 * 60);

        assert!(queue.tick(first - 1).unwrap().is_empty());
        assert_eq!(queue.tick(first).unwrap().len(), 1);
        assert!(queue.tick(first).unwrap().is_empty(), "same minute does not fire twice");

        // Down for an hour: one catch-up run, not twelve.
        assert_eq!(queue.tick(first + 3600).unwrap().len(), 1);
        assert_eq!(queue.metrics("work", first + 3600).unwrap().waiting, 2);
    }
}
