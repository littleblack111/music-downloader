use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct AdaptiveConcurrency {
    current_concurrent: usize,
    min_concurrent: usize,
    max_concurrent: usize,
    failure_streak: usize,
    success_count: usize,
    fail_count: usize,
    semaphore: Arc<Semaphore>,
}

impl AdaptiveConcurrency {
    pub fn new(max_concurrent: usize) -> Self {
        let initial = num_cpus::get().min(max_concurrent);
        AdaptiveConcurrency {
            current_concurrent: initial,
            min_concurrent: 1,
            max_concurrent,
            failure_streak: 0,
            success_count: 0,
            fail_count: 0,
            semaphore: Arc::new(Semaphore::new(initial)),
        }
    }

    pub fn on_success(&mut self) {
        self.success_count += 1;
        self.failure_streak = 0;

        if self.success_count % 10 == 0 && self.current_concurrent < self.max_concurrent {
            self.current_concurrent += 1;
            self.semaphore
                .add_permits(1);
        }
    }

    pub fn on_failure(&mut self) {
        self.fail_count += 1;
        self.failure_streak += 1;

        if self.failure_streak >= 3 && self.current_concurrent > self.min_concurrent {
            let new_concurrent = (self.current_concurrent / 2).max(self.min_concurrent);
            let reduction = self.current_concurrent - new_concurrent;
            self.current_concurrent = new_concurrent;

            for _ in 0..reduction {
                let _ = self
                    .semaphore
                    .try_acquire();
            }
        }
    }

    pub fn get_concurrent(&self) -> usize {
        self.current_concurrent
    }

    pub fn semaphore(&self) -> Arc<Semaphore> {
        self.semaphore
            .clone()
    }
}

pub async fn acquire_slot(semaphore: &Arc<Semaphore>) -> OwnedSemaphorePermit {
    semaphore
        .clone()
        .acquire_owned()
        .await
        .unwrap()
}
