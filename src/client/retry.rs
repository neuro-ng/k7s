use std::time::Duration;

use backoff::backoff::Backoff;
use backoff::ExponentialBackoffBuilder;

/// Build a backoff policy for Kubernetes API reconnects.
///
/// Initial: 500ms, multiplier: 1.5×, max interval: 30s, max elapsed: 2min.
/// These match k9s's reconnect behaviour.
pub fn k8s_reconnect_backoff() -> impl Backoff {
    ExponentialBackoffBuilder::new()
        .with_initial_interval(Duration::from_millis(500))
        .with_multiplier(1.5)
        .with_max_interval(Duration::from_secs(30))
        .with_max_elapsed_time(Some(Duration::from_secs(120)))
        .build()
}

/// Retry an async operation with exponential backoff.
///
/// The operation closure receives no arguments and returns a `Result<T, E>`.
/// Transient errors (indicated by `backoff::Error::Transient`) trigger a
/// retry; permanent errors and exhausted retries surface as `Err`.
pub async fn retry_with_backoff<T, E, Fut>(
    operation: impl Fn() -> Fut,
    mut policy: impl Backoff,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, backoff::Error<E>>>,
    E: std::fmt::Debug,
{
    loop {
        match operation().await {
            Ok(val) => return Ok(val),
            Err(backoff::Error::Permanent(e)) => return Err(e),
            Err(backoff::Error::Transient { err, retry_after }) => {
                let wait = retry_after.or_else(|| policy.next_backoff());
                match wait {
                    Some(d) => {
                        tracing::warn!(error = ?err, wait_ms = d.as_millis(), "transient error, retrying");
                        tokio::time::sleep(d).await;
                    }
                    None => {
                        tracing::error!(error = ?err, "max retries exceeded");
                        return Err(err);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_policy_is_constructable() {
        let mut policy = k8s_reconnect_backoff();
        // The backoff crate applies randomized jitter, so we accept a wide range.
        // The initial interval is 500ms; with ±50% jitter, expect 250ms–750ms.
        let first = policy.next_backoff().unwrap();
        assert!(first <= Duration::from_millis(750), "first interval too large: {first:?}");
        assert!(first >= Duration::from_millis(50), "first interval too small: {first:?}");
    }
}
