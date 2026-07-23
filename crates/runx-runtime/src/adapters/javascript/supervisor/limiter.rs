use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::RuntimeError;

use super::{lock, worker_error};

pub(super) struct InFlightLimiter {
    maximum: usize,
    count: Mutex<usize>,
    available: Condvar,
    peak: AtomicUsize,
}

impl InFlightLimiter {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            maximum,
            count: Mutex::new(0),
            available: Condvar::new(),
            peak: AtomicUsize::new(0),
        }
    }

    pub(super) fn acquire(&self) -> Result<InFlightPermit<'_>, RuntimeError> {
        let mut count = lock(&self.count, "locking JavaScript in-flight limiter")?;
        while *count >= self.maximum {
            count = self.available.wait(count).map_err(|_| {
                worker_error("waiting for JavaScript in-flight capacity: mutex poisoned")
            })?;
        }
        *count += 1;
        self.peak.fetch_max(*count, Ordering::Relaxed);
        Ok(InFlightPermit { limiter: self })
    }

    pub(super) fn peak(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }
}

pub(super) struct InFlightPermit<'a> {
    limiter: &'a InFlightLimiter,
}

impl Drop for InFlightPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut count) = self.limiter.count.lock() {
            *count = count.saturating_sub(1);
            self.limiter.available.notify_one();
        }
    }
}
