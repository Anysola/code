//! Retry schedule for model calls: exponential from 2 s, each gap capped at 10 s, at most 10
//! retries, jittered (±20 %), never shorter than the server's `retry-after` (capped at 15 min) for
//! rate limits and provider failures.

use std::time::Duration;

use rand::Rng;

use super::error::{ApiError, Reason};

const MAX_RETRIES: u32 = 10;
const ONE_SHOT_RETRIES: u32 = 3;
const RETRY_AFTER_MAX: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Default)]
pub struct Retry {
    retries: u32,
}

impl Retry {
    /// Number of the attempt that follows the last scheduled retry (the first call is attempt 1).
    pub fn attempt(&self) -> u32 {
        self.retries + 1
    }

    /// Delay before the next attempt of a step's model call, or `None` to give up.
    pub fn next(&mut self, error: &ApiError) -> Option<Duration> {
        if !error.retryable() || self.retries >= MAX_RETRIES {
            return None;
        }
        Some(self.schedule(error))
    }

    /// Delay before retrying a one-shot generation (title), which only retries calls that did not run.
    pub fn next_one_shot(&mut self, error: &ApiError) -> Option<Duration> {
        if !error.not_run() || self.retries >= ONE_SHOT_RETRIES {
            return None;
        }
        Some(self.schedule(error))
    }

    fn schedule(&mut self, error: &ApiError) -> Duration {
        let base =
            Duration::from_secs(2u64.saturating_pow(self.retries + 1)).min(Duration::from_secs(10));
        self.retries += 1;
        let jittered = base.mul_f64(rand::thread_rng().gen_range(0.8..1.2));
        let minimum = match error.reason {
            Reason::RateLimit | Reason::ProviderInternal => {
                error.retry_after.map(|d| d.min(RETRY_AFTER_MAX))
            }
            _ => None,
        };
        minimum.map_or(jittered, |m| jittered.max(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_bounds() {
        let error = ApiError::http(529, "{}".into(), None, None);
        let mut retry = Retry::default();
        let delays: Vec<Duration> = std::iter::from_fn(|| retry.next(&error)).collect();
        assert_eq!(delays.len(), 10);
        assert!(
            delays[0] >= Duration::from_millis(1600) && delays[0] <= Duration::from_millis(2400)
        );
        assert!(delays.iter().all(|d| *d <= Duration::from_secs(12)));
        let after = ApiError::http(429, "{}".into(), Some(Duration::from_secs(30)), None);
        assert!(Retry::default().next(&after).unwrap() >= Duration::from_secs(30));
        assert!(
            Retry::default()
                .next(&ApiError::http(402, "{}".into(), None, None))
                .is_none()
        );
    }
}
