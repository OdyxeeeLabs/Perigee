use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressurePolicy {
    DropNewest,
    DropOldest,
    Wait,
}

impl Default for BackpressurePolicy {
    fn default() -> Self {
        Self::Wait
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Delivered,
    Dropped,
    WouldBlock,
    TimedOut,
    Closed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackpressureStats {
    pub published: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub timed_out: u64,
    pub closed: u64,
}

struct Queued<T> {
    value: T,
    permit_held: bool,
}

struct Subscriber<T> {
    queue: Mutex<VecDeque<Queued<T>>>,
    capacity: usize,
    permits: Arc<Semaphore>,
    notify: Notify,
    closed: AtomicBool,
}

impl<T> Subscriber<T> {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            permits: Arc::new(Semaphore::new(capacity)),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn close(&self) {
        let _queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.closed.store(true, Ordering::Release);
        self.permits.close();
        self.notify.notify_waiters();
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn enqueue(&self, value: T, permit: OwnedSemaphorePermit) -> bool {
        if self.is_closed() {
            return false;
        }
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.is_closed() || queue.len() >= self.capacity {
            return false;
        }
        std::mem::forget(permit);
        queue.push_back(Queued {
            value,
            permit_held: true,
        });
        self.notify.notify_one();
        true
    }

    fn replace_oldest(&self, value: T) -> (PublishOutcome, bool) {
        if self.is_closed() {
            return (PublishOutcome::Closed, false);
        }
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.is_closed() {
            return (PublishOutcome::Closed, false);
        }
        let mut dropped_oldest = false;
        if queue.len() >= self.capacity {
            dropped_oldest = true;
            if let Some(removed) = queue.pop_front() {
                if removed.permit_held && !self.is_closed() {
                    self.permits.add_permits(1);
                }
            }
        }
        let permit = match self.permits.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return (PublishOutcome::Closed, dropped_oldest),
        };
        std::mem::forget(permit);
        queue.push_back(Queued {
            value,
            permit_held: true,
        });
        self.notify.notify_one();
        (PublishOutcome::Delivered, dropped_oldest)
    }

    fn pop(&self) -> Option<T> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let popped = queue.pop_front()?;
        if popped.permit_held && !self.is_closed() {
            self.permits.add_permits(1);
        }
        drop(queue);
        self.notify.notify_one();
        Some(popped.value)
    }

    fn try_push(&self, value: T, policy: BackpressurePolicy) -> (PublishOutcome, bool) {
        if self.is_closed() {
            return (PublishOutcome::Closed, false);
        }
        match policy {
            BackpressurePolicy::DropOldest => self.replace_oldest(value),
            BackpressurePolicy::DropNewest | BackpressurePolicy::Wait => {
                let permit = match self.permits.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        return (
                            if policy == BackpressurePolicy::Wait {
                                PublishOutcome::WouldBlock
                            } else {
                                PublishOutcome::Dropped
                            },
                            false,
                        );
                    }
                };
                if self.enqueue(value, permit) {
                    (PublishOutcome::Delivered, false)
                } else {
                    (PublishOutcome::Closed, false)
                }
            }
        }
    }

    async fn push_waiting(
        &self,
        value: T,
        timeout: Option<Duration>,
    ) -> PublishOutcome {
        if self.is_closed() {
            return PublishOutcome::Closed;
        }
        let permit = match timeout {
            Some(duration) => match tokio::time::timeout(duration, self.permits.clone().acquire_owned())
                .await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) => return PublishOutcome::Closed,
                Err(_) => return PublishOutcome::TimedOut,
            },
            None => match self.permits.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return PublishOutcome::Closed,
            },
        };
        if self.enqueue(value, permit) {
            PublishOutcome::Delivered
        } else {
            PublishOutcome::Closed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionError {
    Empty,
    Closed,
}

pub struct BoundedSubscription<T> {
    id: usize,
    subscriber: Arc<Subscriber<T>>,
    bus: Weak<BusInner<T>>,
}

impl<T> BoundedSubscription<T> {
    pub async fn recv(&mut self) -> Result<T, SubscriptionError> {
        loop {
            let mut notified = self.subscriber.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(value) = self.subscriber.pop() {
                return Ok(value);
            }
            if self.subscriber.is_closed() {
                return Err(SubscriptionError::Closed);
            }
            notified.await;
        }
    }

    pub fn try_recv(&mut self) -> Result<T, SubscriptionError> {
        self.subscriber.pop().ok_or_else(|| {
            if self.subscriber.is_closed() {
                SubscriptionError::Closed
            } else {
                SubscriptionError::Empty
            }
        })
    }

    pub fn close(&self) {
        self.subscriber.close();
        if let Some(bus) = self.bus.upgrade() {
            bus.remove(self.id);
        }
    }

    pub fn subscriber_id(&self) -> usize {
        self.id
    }
}

impl<T> Drop for BoundedSubscription<T> {
    fn drop(&mut self) {
        if let Some(bus) = self.bus.upgrade() {
            bus.remove(self.id);
        }
    }
}

struct StatsInner {
    published: AtomicU64,
    delivered: AtomicU64,
    dropped: AtomicU64,
    timed_out: AtomicU64,
    closed: AtomicU64,
}

impl StatsInner {
    fn new() -> Self {
        Self {
            published: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            timed_out: AtomicU64::new(0),
            closed: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> BackpressureStats {
        BackpressureStats {
            published: self.published.load(Ordering::Acquire),
            delivered: self.delivered.load(Ordering::Acquire),
            dropped: self.dropped.load(Ordering::Acquire),
            timed_out: self.timed_out.load(Ordering::Acquire),
            closed: self.closed.load(Ordering::Acquire),
        }
    }
}

struct BusInner<T> {
    capacity: usize,
    policy: BackpressurePolicy,
    subscribers: Mutex<HashMap<usize, Arc<Subscriber<T>>>>,
    next_id: AtomicUsize,
    closed: AtomicBool,
    stats: StatsInner,
}

impl<T> BusInner<T> {
    fn remove(&self, id: usize) {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
    }

    fn snapshot_subscribers(&self) -> Vec<Arc<Subscriber<T>>> {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn record_published(&self) {
        self.stats.published.fetch_add(1, Ordering::AcqRel);
    }

    fn record(&self, outcome: PublishOutcome, dropped_oldest: bool) {
        if dropped_oldest {
            self.stats.dropped.fetch_add(1, Ordering::AcqRel);
        }
        match outcome {
            PublishOutcome::Delivered => {
                self.stats.delivered.fetch_add(1, Ordering::AcqRel);
            }
            PublishOutcome::Dropped | PublishOutcome::WouldBlock => {
                self.stats.dropped.fetch_add(1, Ordering::AcqRel);
            }
            PublishOutcome::TimedOut => {
                self.stats.timed_out.fetch_add(1, Ordering::AcqRel);
            }
            PublishOutcome::Closed => {
                self.stats.closed.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

impl<T> Drop for BusInner<T> {
    fn drop(&mut self) {
        let subscribers = self
            .subscribers
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for subscriber in subscribers.values() {
            subscriber.close();
        }
    }
}

pub struct BoundedEventBus<T> {
    inner: Arc<BusInner<T>>,
}

impl<T> Clone for BoundedEventBus<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> BoundedEventBus<T> {
    pub fn new(capacity: usize, policy: BackpressurePolicy) -> Self {
        Self {
            inner: Arc::new(BusInner {
                capacity: capacity.max(1),
                policy,
                subscribers: Mutex::new(HashMap::new()),
                next_id: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
                stats: StatsInner::new(),
            }),
        }
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    pub fn policy(&self) -> BackpressurePolicy {
        self.inner.policy
    }

    pub fn subscribe(&self) -> BoundedSubscription<T> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let subscriber = Arc::new(Subscriber::new(self.inner.capacity));
        if self.inner.closed.load(Ordering::Acquire) {
            subscriber.close();
        } else {
            self.inner
                .subscribers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(id, Arc::clone(&subscriber));
            if self.inner.closed.load(Ordering::Acquire) {
                subscriber.close();
                self.inner.remove(id);
            }
        }
        BoundedSubscription {
            id,
            subscriber,
            bus: Arc::downgrade(&self.inner),
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner
            .subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn publish(&self, value: T) -> PublishOutcome
    where
        T: Clone,
    {
        self.inner.record_published();
        if self.inner.closed.load(Ordering::Acquire) {
            let outcome = PublishOutcome::Closed;
            self.inner.record(outcome, false);
            return outcome;
        }
        let subscribers = self.inner.snapshot_subscribers();
        let mut aggregate = PublishOutcome::Delivered;
        for subscriber in subscribers {
            let (outcome, dropped_oldest) = subscriber.try_push(value.clone(), self.inner.policy);
            self.inner.record(outcome, dropped_oldest);
            if outcome != PublishOutcome::Delivered && aggregate == PublishOutcome::Delivered {
                aggregate = outcome;
            }
        }
        aggregate
    }

    pub fn publish_with_policy(&self, value: T, policy: BackpressurePolicy) -> PublishOutcome
    where
        T: Clone,
    {
        self.inner.record_published();
        if self.inner.closed.load(Ordering::Acquire) {
            let outcome = PublishOutcome::Closed;
            self.inner.record(outcome, false);
            return outcome;
        }
        let subscribers = self.inner.snapshot_subscribers();
        let mut aggregate = PublishOutcome::Delivered;
        for subscriber in subscribers {
            let (outcome, dropped_oldest) = subscriber.try_push(value.clone(), policy);
            self.inner.record(outcome, dropped_oldest);
            if outcome != PublishOutcome::Delivered && aggregate == PublishOutcome::Delivered {
                aggregate = outcome;
            }
        }
        aggregate
    }

    pub async fn publish_async(
        &self,
        value: T,
        timeout: Option<Duration>,
    ) -> PublishOutcome
    where
        T: Clone,
    {
        self.inner.record_published();
        if self.inner.closed.load(Ordering::Acquire) {
            let outcome = PublishOutcome::Closed;
            self.inner.record(outcome, false);
            return outcome;
        }
        let subscribers = self.inner.snapshot_subscribers();
        let mut aggregate = PublishOutcome::Delivered;
        for subscriber in subscribers {
            let (outcome, dropped_oldest) = match self.inner.policy {
                BackpressurePolicy::Wait => {
                    (subscriber.push_waiting(value.clone(), timeout).await, false)
                }
                policy => subscriber.try_push(value.clone(), policy),
            };
            self.inner.record(outcome, dropped_oldest);
            if outcome != PublishOutcome::Delivered && aggregate == PublishOutcome::Delivered {
                aggregate = outcome;
            }
        }
        aggregate
    }

    pub fn stats(&self) -> BackpressureStats {
        self.inner.stats.snapshot()
    }

    pub fn close(&self) {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        for subscriber in self.inner.snapshot_subscribers() {
            subscriber.close();
        }
        self.inner
            .subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }
}
