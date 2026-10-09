//! How a routine's checks and runs have gone, kept by its Runner: when a check last ran and last
//! succeeded, failure streaks by kind, and when to try again. It holds categories, never what a
//! tool said or a credential; the roster carries it, encrypted, to every paired Device.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunPolicy {
    Skip,
    #[default]
    Coalesce,
}

impl MissedRunPolicy {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "skip" => Ok(Self::Skip),
            "coalesce" => Ok(Self::Coalesce),
            _ => Err("Missed-run policy must be skip or coalesce.".into()),
        }
    }

    pub fn should_skip(self, due: i64, now: i64) -> bool {
        self == Self::Skip && now.saturating_sub(due) > 60
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Quiet,
    Ready,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckHealth {
    #[serde(default)]
    pub updated_at: f64,
    pub last_check_at: Option<f64>,
    pub last_success_at: Option<f64>,
    pub status: Option<CheckStatus>,
    #[serde(default)]
    pub connection_failures: u32,
    #[serde(default)]
    pub authentication_failures: u32,
    pub retry_at: Option<f64>,
    /// The runs' own streak with the model provider, which a quiet check does not clear.
    #[serde(default)]
    pub model: ModelHealth,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelHealth {
    pub status: Option<CheckStatus>,
    #[serde(default)]
    pub connection_failures: u32,
    #[serde(default)]
    pub authentication_failures: u32,
    pub retry_at: Option<f64>,
}

impl ModelHealth {
    /// A run failed to reach or sign in to its provider. Other failures (a refusal, a bad
    /// answer) show as the run's outcome and leave the streak alone.
    pub fn record_failure(&mut self, at: f64, failure: Failure) {
        if !matches!(failure, Failure::Connection | Failure::Authentication) {
            return;
        }
        let mut health = CheckHealth {
            connection_failures: self.connection_failures,
            authentication_failures: self.authentication_failures,
            ..Default::default()
        };
        health.record(at, false, Some(failure));
        self.status = health.status;
        self.connection_failures = health.connection_failures;
        self.authentication_failures = health.authentication_failures;
        self.retry_at = health.retry_at;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Connection,
    Authentication,
    Blocked,
    Script,
}

/// External tool errors use several transports. Only their category survives in health;
/// scripts may still inspect the original error in their own turn.
pub fn classify(error: &str) -> Failure {
    let error = error.to_lowercase();
    if [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "authentication",
        "needs_auth",
        "sign in",
        "sign-in",
        "token expired",
        "invalid token",
        "not connected",
    ]
    .iter()
    .any(|word| error.contains(word))
    {
        Failure::Authentication
    } else if [
        "connection",
        "connect error",
        "timed out",
        "timeout",
        "unavailable",
        "502",
        "503",
        "504",
        "dns",
        "network",
    ]
    .iter()
    .any(|word| error.contains(word))
    {
        Failure::Connection
    } else if [
        "can change things",
        "only looks",
        "access refused",
        "blocked",
        "permission",
        "read-only",
    ]
    .iter()
    .any(|word| error.contains(word))
    {
        Failure::Blocked
    } else {
        Failure::Script
    }
}

impl CheckHealth {
    /// The later of the check's and the runs' retry times, while either backs off.
    pub fn retry_at(&self) -> Option<f64> {
        self.retry_at.into_iter().chain(self.model.retry_at).reduce(f64::max)
    }

    pub fn record(&mut self, at: f64, found: bool, failure: Option<Failure>) {
        self.updated_at = at;
        self.last_check_at = Some(at);
        self.retry_at = None;
        match failure {
            None => {
                self.last_success_at = Some(at);
                self.status = Some(if found {
                    CheckStatus::Ready
                } else {
                    CheckStatus::Quiet
                });
                self.connection_failures = 0;
                self.authentication_failures = 0;
            }
            Some(Failure::Authentication) => {
                self.connection_failures = 0;
                self.authentication_failures = self.authentication_failures.saturating_add(1);
                self.status = Some(if self.authentication_failures >= 3 {
                    CheckStatus::Blocked
                } else {
                    CheckStatus::Failed
                });
                self.retry_at = Some(at + backoff(self.authentication_failures) as f64);
            }
            Some(Failure::Connection) => {
                self.authentication_failures = 0;
                self.connection_failures = self.connection_failures.saturating_add(1);
                self.status = Some(CheckStatus::Failed);
                self.retry_at = Some(at + backoff(self.connection_failures) as f64);
            }
            Some(failure) => {
                self.connection_failures = 0;
                self.authentication_failures = 0;
                self.status = Some(if failure == Failure::Blocked {
                    CheckStatus::Blocked
                } else {
                    CheckStatus::Failed
                });
            }
        }
    }

    pub fn resume(&mut self) {
        self.connection_failures = 0;
        self.authentication_failures = 0;
        self.retry_at = None;
        self.status = None;
        self.model = Default::default();
    }
}

/// Five minutes initially, doubles to a six-hour cap, persisted so restarts do not reset it.
fn backoff(failures: u32) -> i64 {
    (300_i64 << failures.saturating_sub(1).min(7)).min(6 * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_backoff_is_bounded_and_success_resets_it() {
        let mut health = CheckHealth::default();
        health.record(100.0, false, None);
        for (attempt, wait) in [(1, 300.0), (2, 600.0), (3, 1200.0), (4, 2400.0)] {
            health.record(200.0, false, Some(Failure::Connection));
            assert_eq!(health.connection_failures, attempt);
            assert_eq!(health.retry_at, Some(200.0 + wait));
            assert_eq!(health.last_success_at, Some(100.0));
        }
        for _ in 0..50 {
            health.record(200.0, false, Some(Failure::Connection));
        }
        assert_eq!(health.retry_at, Some(200.0 + 21_600.0));
        health.record(500.0, false, None);
        assert_eq!(health.status, Some(CheckStatus::Quiet));
        assert_eq!(health.last_success_at, Some(500.0));
        assert_eq!((health.connection_failures, health.retry_at), (0, None));
    }

    #[test]
    fn repeated_authentication_failure_blocks_with_recovery() {
        let mut health = CheckHealth::default();
        for attempt in 1..=3 {
            health.record(
                100.0,
                false,
                Some(classify("HTTP 401 Unauthorized: expired token")),
            );
            assert_eq!(health.authentication_failures, attempt);
        }
        assert_eq!(health.status, Some(CheckStatus::Blocked));
        health.resume();
        assert_eq!(
            (
                health.authentication_failures,
                health.retry_at,
                health.status
            ),
            (0, None, None)
        );
        assert_eq!(
            classify("connect error: connection refused"),
            Failure::Connection
        );
        assert_eq!(
            classify("write can change things, and a check only looks"),
            Failure::Blocked
        );
        assert_eq!(
            classify("ReferenceError: name is undefined"),
            Failure::Script
        );
    }

    #[test]
    fn quiet_checks_do_not_reset_model_authentication_failures() {
        let mut health = CheckHealth::default();
        for attempt in 1..=3 {
            health.record(attempt as f64, false, None);
            health
                .model
                .record_failure(attempt as f64, Failure::Authentication);
        }
        assert_eq!(health.status, Some(CheckStatus::Quiet));
        assert_eq!(health.model.status, Some(CheckStatus::Blocked));
        assert_eq!(health.model.authentication_failures, 3);
        let before = health.model.clone();
        health.model.record_failure(4.0, classify("The model refused the request"));
        assert_eq!(health.model, before, "only connection and sign-in failures count for a run");
    }
}
