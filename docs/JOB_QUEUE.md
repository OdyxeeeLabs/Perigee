# Job queue

`core/src/job_queue.rs` is a background job queue with priorities, scheduling, retries and
a dead-letter list. (`jobs.rs` remains the tracker for long-running simulation jobs.)

## Using it

```rust
let queue = JobQueue::new().with_dead_letter_alert(|job| tracing::error!(job = job.id, "job died"));
queue.configure("emails", QueueConfig { concurrency: 8, rate_limit: Some(RateLimit { max_starts: 100, per_secs: 60 }), ..Default::default() })?;

queue.enqueue(JobSpec::new("emails", "send", json!({"to": "a@b.c"})).priority(Priority::High), now)?;
queue.schedule_cron("nightly-reconcile", "0 2 * * *", JobSpec::new("maintenance", "reconcile", json!({})), now)?;

// worker loop
queue.tick(now)?;                                   // enqueue due cron jobs
queue.work_once("emails", now, |job| do_work(job))?; // claim -> handler -> complete/fail
```

## Behaviour

* **Priorities**: `High` before `Normal` before `Low`; first-in first-out within each.
* **Delays and cron**: `run_at` delays a job; cron is five fields in UTC (`*`, lists, ranges,
  `/step`). Runs missed while the process was down collapse into one.
* **Retries**: `max_attempts` (default 5) with `Fixed` or `Exponential` backoff (default 5s
  doubling, capped at 1h). When attempts run out the job becomes `Dead` and the alert fires.
* **Concurrency and rate limits** are per queue. `claim` returns `None` when either is hit.
* **Progress**: `report_progress(id, percent, message)` while a job runs.

## Dashboard

`metrics(queue, now)` returns waiting/delayed/active/completed/dead counts, throughput over
the last minute, average duration, oldest wait and a `health` verdict (`Degraded` when jobs
are dead or have waited longer than `stale_wait_secs`). `history(queue, n)` lists recent
finished jobs, `dead_letters(queue)` the dead ones, and `retry_dead(id, now)` is the manual
retry button.

## Persistence

`JobQueue` keeps state in memory. To survive restarts, persist the `Job` records (they are
`Serialize`/`Deserialize`) in Redis or SQL and rebuild the queue on boot; the `jobs` table in
`core/migrations/001_create_jobs_table.sql` is a starting point.
