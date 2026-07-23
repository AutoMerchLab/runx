use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::RuntimeError;

use super::session::WorkerSession;
use super::{lock, worker_error};

pub(super) struct WorkerPool {
    state: Mutex<PoolState>,
    available: Condvar,
    maximum: usize,
    spawn_count: AtomicU64,
    peak_active: AtomicUsize,
}

#[derive(Default)]
struct PoolState {
    slots: Vec<PoolSlot>,
    starting: usize,
    active: usize,
}

struct PoolSlot {
    session: Arc<WorkerSession>,
    busy: bool,
}

impl WorkerPool {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            state: Mutex::new(PoolState::default()),
            available: Condvar::new(),
            maximum,
            spawn_count: AtomicU64::new(0),
            peak_active: AtomicUsize::new(0),
        }
    }

    pub(super) fn acquire(&self) -> Result<WorkerLease<'_>, RuntimeError> {
        loop {
            let mut state = lock(&self.state, "locking JavaScript worker pool")?;
            if let Some(index) = state.slots.iter().position(|slot| !slot.busy) {
                state.slots[index].busy = true;
                let session = state.slots[index].session.clone();
                self.record_active(&mut state);
                return Ok(WorkerLease::new(self, session));
            }
            if state.slots.len().saturating_add(state.starting) < self.maximum {
                state.starting = state.starting.saturating_add(1);
                drop(state);
                let started = WorkerSession::start();
                let mut state = lock(&self.state, "recording JavaScript worker startup")?;
                state.starting = state.starting.saturating_sub(1);
                match started {
                    Ok(session) => {
                        let session = Arc::new(session);
                        state.slots.push(PoolSlot {
                            session: session.clone(),
                            busy: true,
                        });
                        self.spawn_count.fetch_add(1, Ordering::Relaxed);
                        self.record_active(&mut state);
                        self.available.notify_all();
                        return Ok(WorkerLease::new(self, session));
                    }
                    Err(error) => {
                        self.available.notify_all();
                        return Err(error);
                    }
                }
            }
            state = self.available.wait(state).map_err(|_| {
                worker_error("waiting for JavaScript worker capacity: mutex poisoned")
            })?;
            drop(state);
        }
    }

    fn record_active(&self, state: &mut PoolState) {
        state.active = state.active.saturating_add(1);
        self.peak_active.fetch_max(state.active, Ordering::Relaxed);
    }

    fn release(&self, session: &Arc<WorkerSession>, reusable: bool) {
        if !reusable {
            session.terminate();
        }
        if let Ok(mut state) = self.state.lock() {
            state.active = state.active.saturating_sub(1);
            if let Some(index) = state
                .slots
                .iter()
                .position(|slot| Arc::ptr_eq(&slot.session, session))
            {
                if reusable {
                    state.slots[index].busy = false;
                } else {
                    state.slots.swap_remove(index);
                }
            }
            self.available.notify_one();
        }
    }

    pub(super) fn spawn_count(&self) -> u64 {
        self.spawn_count.load(Ordering::Relaxed)
    }

    pub(super) fn peak_active(&self) -> usize {
        self.peak_active.load(Ordering::Relaxed)
    }
}

pub(super) struct WorkerLease<'a> {
    pool: &'a WorkerPool,
    session: Arc<WorkerSession>,
    reusable: bool,
}

impl<'a> WorkerLease<'a> {
    fn new(pool: &'a WorkerPool, session: Arc<WorkerSession>) -> Self {
        Self {
            pool,
            session,
            reusable: false,
        }
    }

    pub(super) fn session(&self) -> &WorkerSession {
        &self.session
    }

    pub(super) fn mark_reusable(&mut self) {
        self.reusable = true;
    }
}

impl Drop for WorkerLease<'_> {
    fn drop(&mut self) {
        self.pool.release(&self.session, self.reusable);
    }
}
