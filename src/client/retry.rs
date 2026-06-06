use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};

/// Build a backoff policy for Kubernetes API reconnects.
///
/// Initial: 500ms, multiplier: 1.5×, max interval: 30s, max retries: 10 (~2min budget).
/// These match k9s's reconnect behaviour.
pub fn k8s_reconnect_backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_factor(1.5)
        .with_max_delay(Duration::from_secs(30))
        .with_max_times(10)
        .with_jitter()
}

/// Retry an async operation with exponential backoff.
///
/// The operation is retried on any `Err`. Returns the final error when
/// the backoff policy is exhausted.
pub async fn retry_with_backoff<T, E, Fut>(
    operation: impl FnMut() -> Fut,
    backoff: ExponentialBuilder,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    operation.retry(backoff).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_policy_is_constructable() {
        // Verify the builder constructs without panicking.
        let builder = k8s_reconnect_backoff();
        let _ = builder;
    }
}
