//! Retry utilities for resilient agent spawning.
//!
//! This module provides retry logic with exponential backoff for operations
//! that may fail transiently, such as MCP server startup or ACP session creation.

use anyhow::{Context, Result};
use std::future::Future;
use std::time::Duration;

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the initial attempt).
    pub max_retries: u32,
    /// Initial delay between retries.
    pub initial_delay: Duration,
    /// Maximum delay between retries (caps exponential backoff).
    pub max_delay: Duration,
    /// Multiplier for exponential backoff (e.g., 2.0 doubles the delay each retry).
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Create a retry config from environment variables.
    ///
    /// Environment variables:
    /// - `PAPERBOAT_SPAWN_RETRIES`: Max retry attempts (default: 3)
    /// - `PAPERBOAT_SPAWN_RETRY_DELAY_MS`: Initial delay in ms (default: 500)
    pub fn from_env() -> Self {
        let max_retries = std::env::var("PAPERBOAT_SPAWN_RETRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);

        let initial_delay_ms = std::env::var("PAPERBOAT_SPAWN_RETRY_DELAY_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(500);

        Self {
            max_retries,
            initial_delay: Duration::from_millis(initial_delay_ms),
            ..Default::default()
        }
    }

    /// Create a config with no retries (useful for testing).
    #[allow(dead_code)]
    pub const fn no_retry() -> Self {
        Self {
            max_retries: 0,
            initial_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            backoff_multiplier: 1.0,
        }
    }
}

/// Check if an error is likely transient and worth retrying.
///
/// We consider errors transient if they relate to:
/// - MCP server startup failures
/// - Connection/timeout issues
/// - Process spawning issues
pub fn is_transient_error(error: &anyhow::Error) -> bool {
    let error_str = format!("{error:#}").to_lowercase();

    // MCP server startup errors are the primary target for retries
    if error_str.contains("mcp server startup")
        || error_str.contains("mcp")
        || error_str.contains("failed to spawn")
        || error_str.contains("failed to initialize")
        || error_str.contains("failed to create acp session")
        || error_str.contains("timeout")
        || error_str.contains("connection refused")
        || error_str.contains("broken pipe")
        || error_str.contains("resource temporarily unavailable")
    {
        return true;
    }

    false
}

/// Check if an error indicates a model is not available.
///
/// This is used to trigger fallback to the next model in the chain.
/// We detect phrases like "Cannot use this model" from Cursor.
pub fn is_model_not_available_error(error: &anyhow::Error) -> bool {
    let error_str = format!("{error:#}").to_lowercase();

    error_str.contains("cannot use this model")
        || error_str.contains("model not found")
        || error_str.contains("model not available")
        || error_str.contains("invalid model")
        || error_str.contains("unknown model")
        || error_str.contains("unsupported model")
}

/// Execute an async operation with retry logic.
///
/// # Arguments
/// * `config` - Retry configuration
/// * `operation_name` - Human-readable name for logging
/// * `operation` - Async closure that produces the operation's result
///
/// # Returns
/// The result of the operation, or the last error if all retries fail.
pub async fn retry_async<T, F, Fut>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;

    loop {
        attempt += 1;

        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    tracing::info!(
                        "🔄 {} succeeded on attempt {}/{}",
                        operation_name,
                        attempt,
                        config.max_retries + 1
                    );
                }
                return Ok(result);
            }
            Err(e) => {
                let is_transient = is_transient_error(&e);
                let can_retry = attempt <= config.max_retries && is_transient;

                if can_retry {
                    tracing::warn!(
                        "⚠️ {} failed (attempt {}/{}): {}. Retrying in {:?}...",
                        operation_name,
                        attempt,
                        config.max_retries + 1,
                        e,
                        delay
                    );
                    tokio::time::sleep(delay).await;

                    // Exponential backoff with cap
                    delay = Duration::from_secs_f64(
                        (delay.as_secs_f64() * config.backoff_multiplier)
                            .min(config.max_delay.as_secs_f64()),
                    );
                } else {
                    // Either exhausted retries or error is not transient
                    let reason = if is_transient {
                        "exhausted retries"
                    } else {
                        "non-transient error"
                    };
                    tracing::error!(
                        "❌ {operation_name} failed after {attempt} attempt(s) ({reason}): {e:#}",
                    );
                    return Err(e).with_context(|| {
                        format!("{operation_name} failed after {attempt} attempt(s)")
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay, Duration::from_millis(500));
    }

    #[test]
    fn test_is_transient_error() {
        // MCP server startup errors should be transient
        let e = anyhow::anyhow!("MCP server startup error: connection refused");
        assert!(is_transient_error(&e));

        // Timeout errors should be transient
        let e = anyhow::anyhow!("Failed to spawn auggie: timeout");
        assert!(is_transient_error(&e));

        // Generic errors should not be transient
        let e = anyhow::anyhow!("Invalid configuration: missing field 'name'");
        assert!(!is_transient_error(&e));
    }

    #[tokio::test]
    async fn test_retry_succeeds_immediately() {
        let config = RetryConfig::default();
        let result = retry_async(&config, "test_op", || async { Ok::<_, anyhow::Error>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let config = RetryConfig {
            max_retries: 3,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            backoff_multiplier: 2.0,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = retry_async(&config, "test_op", || {
            let counter = counter_clone.clone();
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(anyhow::anyhow!("MCP server startup failed"))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig {
            max_retries: 2,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            backoff_multiplier: 2.0,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = retry_async(&config, "test_op", || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(anyhow::anyhow!("MCP server startup failed"))
            }
        })
        .await;

        assert!(result.is_err());
        // Initial attempt + 2 retries = 3 attempts
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_non_transient_error_no_retry() {
        let config = RetryConfig {
            max_retries: 3,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            backoff_multiplier: 2.0,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = retry_async(&config, "test_op", || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(anyhow::anyhow!("Invalid configuration"))
            }
        })
        .await;

        assert!(result.is_err());
        // Non-transient error should not retry
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
