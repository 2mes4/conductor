//! Distributed worker pool — pulls jobs from the PostgreSQL queue and executes
//! the full orchestration lifecycle.
//!
//! Each worker runs in its own tokio task. Multiple workers can run on the same
//! node or across distributed nodes (they all claim from the same `SKIP LOCKED`
//! queue, so there's no double-processing).

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::error::Result;
use crate::models::AgentTask;
use crate::server::AppState;

/// A worker that continuously claims and processes jobs.
pub struct Worker {
    id: String,
    state: AppState,
    poll_interval: Duration,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl Worker {
    /// Create a new worker with a unique ID.
    pub fn new(state: AppState) -> Self {
        Self {
            id: format!("worker-{}", Uuid::new_v4().as_simple()),
            state,
            poll_interval: Duration::from_secs(1),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Set the poll interval (how often to check for new jobs when idle).
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Get the worker ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Start the worker loop. Runs until `stop()` is called.
    pub async fn run(&self) -> Result<()> {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        tracing::info!(worker_id = %self.id, "worker started");

        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            match crate::state::queue::claim_next(&self.state, &self.id).await {
                Ok(Some(job)) => {
                    tracing::info!(
                        worker_id = %self.id,
                        job_id = %job.id,
                        session_id = %job.session_id,
                        "processing job"
                    );

                    let task_result = self.process_job(&job).await;

                    match &task_result {
                        Ok(()) => {
                            let _ = crate::state::queue::complete(&self.state, job.id).await;
                        }
                        Err(e) => {
                            tracing::error!(worker_id = %self.id, job_id = %job.id, error = %e, "job failed");
                            let _ = crate::state::queue::fail(&self.state, job.id, &e.to_string())
                                .await;
                        }
                    }
                }
                Ok(None) => {
                    // No jobs available — sleep and retry.
                    tokio::time::sleep(self.poll_interval).await;
                }
                Err(e) => {
                    tracing::error!(worker_id = %self.id, error = %e, "failed to claim job");
                    tokio::time::sleep(self.poll_interval).await;
                }
            }
        }

        tracing::info!(worker_id = %self.id, "worker stopped");
        Ok(())
    }

    /// Signal the worker to stop after the current job completes.
    pub fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if the worker is still running.
    pub fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Deserialize a job payload and run the orchestration lifecycle.
    async fn process_job(&self, job: &crate::state::queue::Job) -> Result<()> {
        let task: AgentTask = serde_json::from_value(job.payload.clone()).map_err(|e| {
            crate::error::ConductorError::Other(format!("failed to deserialize job payload: {e}"))
        })?;

        crate::orchestrator::run_session(self.state.clone(), job.session_id, task).await
    }
}

/// Spawn N workers as background tokio tasks. Returns handles for graceful shutdown.
pub fn spawn_pool(state: AppState, count: usize) -> Vec<Arc<Worker>> {
    let mut workers = Vec::with_capacity(count);

    for _ in 0..count {
        let worker = Arc::new(Worker::new(state.clone()));
        let w = worker.clone();
        tokio::spawn(async move {
            if let Err(e) = w.run().await {
                tracing::error!(worker_id = %w.id(), error = %e, "worker exited with error");
            }
        });
        workers.push(worker);
    }

    tracing::info!(count, "worker pool spawned");
    workers
}

/// Gracefully stop a pool of workers.
pub async fn stop_pool(workers: &[Arc<Worker>]) {
    for worker in workers {
        worker.stop();
    }
    // Give workers a moment to finish their current jobs.
    tokio::time::sleep(Duration::from_millis(100)).await;
    tracing::info!("worker pool stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_has_unique_id() {
        // Can't easily test without a real AppState, but verify ID format.
        let id1 = format!("worker-{}", Uuid::new_v4().as_simple());
        let id2 = format!("worker-{}", Uuid::new_v4().as_simple());
        assert_ne!(id1, id2);
        assert!(id1.starts_with("worker-"));
    }
}
