//! A deadline and cancellation owner shared by every step of a native probe.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct ProbeBudget {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}

impl ProbeBudget {
    pub fn new(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn finished(&self) -> bool {
        self.cancelled() || self.expired()
    }
}
