//! Metrics collection for monitoring agent performance and system health.
//!
//! This module provides metrics collection using the `metrics` crate. Metrics are only
//! recorded when the `metrics` feature is enabled and `init_metrics()` has been called.
//!
//! # Available Metrics
//!
//! ## Counters
//! - `paperboat_agents_spawned_total` - Total agents spawned (labels: `agent_type`)
//! - `paperboat_tasks_total` - Total task status transitions (labels: `status`)
//! - `paperboat_tool_calls_total` - Total tool calls made (labels: `tool_name`)
//!
//! ## Histograms
//! - `paperboat_agent_duration_seconds` - Agent execution duration (labels: `agent_type`, `success`)
//!
//! # Usage
//!
//! Metrics are only recorded when both conditions are met:
//! 1. The crate is compiled with the `metrics` feature
//! 2. `init_metrics()` is called at startup (e.g., when `--metrics` flag is passed)
//!
//! If either condition is not met, all metric operations are no-ops with zero overhead.

/// Whether metrics collection is currently enabled.
/// This is controlled by calling `init_metrics()` at startup.
#[cfg(feature = "metrics")]
static METRICS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Initialize the metrics system.
///
/// This sets up the metrics recorder. Must be called before any metrics will be recorded.
/// If `enable_prometheus` is true, starts a Prometheus HTTP exporter on the specified port.
///
/// # Arguments
/// * `enable_prometheus` - Whether to start the Prometheus HTTP exporter
/// * `prometheus_port` - Port for the Prometheus exporter (default: 9090)
#[cfg(feature = "metrics")]
pub fn init_metrics(enable_prometheus: bool, prometheus_port: u16) -> anyhow::Result<()> {
    use metrics_exporter_prometheus::PrometheusBuilder;

    if enable_prometheus {
        let addr = std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            prometheus_port,
        );

        PrometheusBuilder::new()
            .with_http_listener(addr)
            .install()
            .map_err(|e| anyhow::anyhow!("Failed to install Prometheus exporter: {e}"))?;

        tracing::info!("📊 Prometheus metrics exporter started on http://{addr}");
    } else {
        // Install a no-op recorder that still allows metrics to be recorded
        // (they just won't be exported anywhere)
        PrometheusBuilder::new()
            .install()
            .map_err(|e| anyhow::anyhow!("Failed to install metrics recorder: {e}"))?;

        tracing::info!("📊 Metrics collection enabled (no exporter)");
    }

    METRICS_ENABLED.store(true, std::sync::atomic::Ordering::Release);
    describe_metrics();
    Ok(())
}

/// No-op metrics initialization when the feature is disabled.
#[cfg(not(feature = "metrics"))]
#[allow(clippy::unnecessary_wraps)]
pub fn init_metrics(_enable_prometheus: bool, _prometheus_port: u16) -> anyhow::Result<()> {
    tracing::debug!("Metrics feature not enabled, skipping initialization");
    Ok(())
}

/// Describe all metrics with their help text.
#[cfg(feature = "metrics")]
fn describe_metrics() {
    use metrics::{describe_counter, describe_histogram, Unit};

    describe_counter!(
        "paperboat_agents_spawned_total",
        Unit::Count,
        "Total number of agents spawned"
    );

    describe_histogram!(
        "paperboat_agent_duration_seconds",
        Unit::Seconds,
        "Duration of agent execution in seconds"
    );

    describe_counter!(
        "paperboat_tasks_total",
        Unit::Count,
        "Total number of task status transitions"
    );

    describe_counter!(
        "paperboat_tool_calls_total",
        Unit::Count,
        "Total number of tool calls made by agents"
    );
}

/// Record an agent spawn event.
///
/// # Arguments
/// * `agent_type` - The type/role of the agent (e.g., "implementer", "verifier")
#[cfg(feature = "metrics")]
pub fn record_agent_spawned(agent_type: &str) {
    if !METRICS_ENABLED.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    metrics::counter!("paperboat_agents_spawned_total", "agent_type" => agent_type.to_owned())
        .increment(1);
}

#[cfg(not(feature = "metrics"))]
#[allow(clippy::missing_const_for_fn)]
pub fn record_agent_spawned(_agent_type: &str) {}

/// Record agent completion with duration.
///
/// # Arguments
/// * `agent_type` - The type/role of the agent
/// * `success` - Whether the agent completed successfully
/// * `duration` - How long the agent took to complete
#[cfg(feature = "metrics")]
pub fn record_agent_completed(agent_type: &str, success: bool, duration: std::time::Duration) {
    if !METRICS_ENABLED.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let success_str = if success { "true" } else { "false" };
    metrics::histogram!(
        "paperboat_agent_duration_seconds",
        "agent_type" => agent_type.to_owned(),
        "success" => success_str.to_owned()
    )
    .record(duration.as_secs_f64());
}

#[cfg(not(feature = "metrics"))]
#[allow(clippy::missing_const_for_fn)]
pub fn record_agent_completed(_agent_type: &str, _success: bool, _duration: std::time::Duration) {}

/// Record a task status transition.
///
/// # Arguments
/// * `status` - The new status of the task (e.g., "pending", "completed", "failed")
#[cfg(feature = "metrics")]
pub fn record_task_status(status: &str) {
    if !METRICS_ENABLED.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    metrics::counter!("paperboat_tasks_total", "status" => status.to_owned()).increment(1);
}

#[cfg(not(feature = "metrics"))]
#[allow(clippy::missing_const_for_fn)]
pub fn record_task_status(_status: &str) {}

/// Record a tool call.
///
/// # Arguments
/// * `tool_name` - The name of the tool that was called
#[cfg(feature = "metrics")]
pub fn record_tool_call(tool_name: &str) {
    if !METRICS_ENABLED.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    metrics::counter!("paperboat_tool_calls_total", "tool_name" => tool_name.to_owned())
        .increment(1);
}

#[cfg(not(feature = "metrics"))]
#[allow(clippy::missing_const_for_fn)]
pub fn record_tool_call(_tool_name: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_functions_are_callable() {
        // These should all be no-ops when metrics are not initialized
        // Just verify they don't panic
        record_agent_spawned("implementer");
        record_agent_completed("implementer", true, std::time::Duration::from_secs(10));
        record_task_status("completed");
        record_tool_call("complete");
    }
}
