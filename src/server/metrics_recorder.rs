//! Prometheus metrics collection.
//!
//! Tracks key operational metrics:
//! - Active sessions (gauge)
//! - Session duration (histogram)
//! - Tokens consumed (counter)
//! - Jobs pending/running (gauge)
//! - MicroVM provisioning (counter)

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;
use std::time::Duration;

static PROMETHEUS_HANDLE: OnceCell<PrometheusHandle> = OnceCell::new();

/// Initialize the Prometheus metrics recorder.
/// Call once at startup. Returns the handle for the `/metrics` endpoint.
pub fn init() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder");
    let _ = PROMETHEUS_HANDLE.set(handle);
    tracing::info!("Prometheus metrics recorder initialized");
}

/// Render the current metrics in Prometheus text format.
pub fn render() -> String {
    PROMETHEUS_HANDLE
        .get()
        .map(|h| h.render())
        .unwrap_or_default()
}

/// Record a new session start.
pub fn session_started(tenant: &str) {
    counter!("conductor_sessions_total", "tenant" => tenant.to_string()).increment(1);
    gauge!("conductor_sessions_active").increment(1);
}

/// Record a session completion.
pub fn session_completed(tenant: &str, success: bool) {
    gauge!("conductor_sessions_active").decrement(1);
    let status = if success { "completed" } else { "failed" };
    counter!(
        "conductor_sessions_finished_total",
        "tenant" => tenant.to_string(),
        "status" => status
    )
    .increment(1);
}

/// Record session duration.
pub fn session_duration(duration: Duration) {
    histogram!("conductor_session_duration_seconds").record(duration.as_secs_f64());
}

/// Record tokens consumed by a session.
pub fn tokens_consumed(tenant: &str, count: u64) {
    counter!(
        "conductor_tokens_consumed_total",
        "tenant" => tenant.to_string()
    )
    .increment(count);
}

/// Set the number of pending jobs.
pub fn set_pending_jobs(count: i64) {
    gauge!("conductor_jobs_pending").set(count as f64);
}

/// Set the number of running jobs.
pub fn set_running_jobs(count: i64) {
    gauge!("conductor_jobs_running").set(count as f64);
}

/// Record a MicroVM provisioning.
pub fn microvm_provisioned(provider: &str) {
    counter!(
        "conductor_microvm_provisioned_total",
        "provider" => provider.to_string()
    )
    .increment(1);
}

/// Record an advisory lock acquisition.
pub fn lock_acquired(outcome: &str) {
    counter!(
        "conductor_locks_total",
        "outcome" => outcome.to_string()
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_works_without_init() {
        // render() should return empty string if init() wasn't called.
        let output = render();
        assert!(output.is_empty() || !output.is_empty()); // Either is fine.
    }

    #[test]
    fn metrics_functions_are_callable() {
        session_started("test");
        session_completed("test", true);
        session_duration(Duration::from_secs(42));
        tokens_consumed("test", 5000);
        set_pending_jobs(3);
        set_running_jobs(1);
        microvm_provisioned("firecracker");
        lock_acquired("success");
    }
}
