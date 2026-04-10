//! Token usage tracking and budget enforcement.
//!
//! Prevents runaway API costs by tracking token consumption per session and
//! per query, warning at a configurable threshold, and hard-capping at the
//! session maximum.

use crate::config::TokenBudgetConfig;

/// Token budget tracker for a single AI session.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    used_session: u32,
    max_session:  u32,
    max_query:    u32,
    warn_at:      u32,
}

/// The outcome of a budget check before sending a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetCheck {
    /// Under limit — proceed.
    Ok,
    /// Approaching the warning threshold.
    Warning { used: u32, max: u32 },
    /// Session budget exhausted — query must be rejected.
    Exhausted,
    /// Query payload exceeds the per-query limit.
    QueryTooLarge { tokens: u32, limit: u32 },
}

impl TokenBudget {
    pub fn from_config(cfg: &TokenBudgetConfig) -> Self {
        Self {
            used_session: 0,
            max_session:  cfg.max_per_session,
            max_query:    cfg.max_per_query,
            warn_at:      cfg.warn_at,
        }
    }

    /// Check whether a query of `query_tokens` estimated tokens is within budget.
    pub fn check(&self, query_tokens: u32) -> BudgetCheck {
        if query_tokens > self.max_query {
            return BudgetCheck::QueryTooLarge {
                tokens: query_tokens,
                limit:  self.max_query,
            };
        }
        if self.used_session >= self.max_session {
            return BudgetCheck::Exhausted;
        }
        if self.used_session + query_tokens >= self.warn_at {
            return BudgetCheck::Warning {
                used: self.used_session,
                max:  self.max_session,
            };
        }
        BudgetCheck::Ok
    }

    /// Record that `tokens` were used in the last exchange.
    pub fn record_usage(&mut self, tokens: u32) {
        self.used_session = self.used_session.saturating_add(tokens);
    }

    pub fn used(&self) -> u32 { self.used_session }
    pub fn remaining(&self) -> u32 { self.max_session.saturating_sub(self.used_session) }
    pub fn max_session(&self) -> u32 { self.max_session }

    /// Percentage of session budget consumed (0–100).
    pub fn usage_pct(&self) -> u8 {
        if self.max_session == 0 { return 100; }
        ((self.used_session as f64 / self.max_session as f64) * 100.0) as u8
    }
}

/// Estimate the token count of a string using a simple heuristic.
///
/// Real tokenisation (via tiktoken-rs) can be done in the AI session;
/// this fast estimate is used for budget checks before network calls.
///
/// Heuristic: ~4 characters per token (OpenAI rule of thumb).
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f64 / 4.0).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TokenBudgetConfig;

    fn budget() -> TokenBudget {
        TokenBudget::from_config(&TokenBudgetConfig {
            max_per_session: 100_000,
            max_per_query:   4_000,
            warn_at:         80_000,
        })
    }

    #[test]
    fn check_ok_under_limits() {
        assert_eq!(budget().check(100), BudgetCheck::Ok);
    }

    #[test]
    fn check_query_too_large() {
        assert_eq!(
            budget().check(5_000),
            BudgetCheck::QueryTooLarge { tokens: 5_000, limit: 4_000 }
        );
    }

    #[test]
    fn check_exhausted_after_max() {
        let mut b = budget();
        b.record_usage(100_000);
        assert_eq!(b.check(1), BudgetCheck::Exhausted);
    }

    #[test]
    fn check_warning_at_threshold() {
        let mut b = budget();
        b.record_usage(80_000);
        assert!(matches!(b.check(100), BudgetCheck::Warning { .. }));
    }

    #[test]
    fn usage_pct_scales_correctly() {
        let mut b = budget();
        b.record_usage(50_000);
        assert_eq!(b.usage_pct(), 50);
    }

    #[test]
    fn estimate_tokens_nonempty() {
        assert!(estimate_tokens("hello world") > 0);
        assert!(estimate_tokens("") == 0);
    }
}
