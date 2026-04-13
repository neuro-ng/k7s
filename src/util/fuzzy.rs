//! Fuzzy substring matching for command-prompt autocomplete — Phase 5.14.
//!
//! Uses a simple but effective scoring model:
//!
//! - **Prefix match** (highest score) — candidate starts with the query
//! - **Word-boundary match** — query letters appear at word/separator boundaries
//! - **Subsequence match** — every query character appears in order somewhere in the candidate
//!
//! Returns candidates sorted best-score first.  Designed to run on small sets
//! (tens of aliases), so no exotic data structures are needed.
//!
//! # k9s Reference
//! `internal/model/fish_buff.go` — suggestion buffer driven by fuzzy search.

/// A scored match result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch<'a> {
    /// The matching candidate string.
    pub candidate: &'a str,
    /// Score: higher is a better match.  Negative means "did not match".
    pub score: i32,
}

/// Match `query` against a slice of `candidates` and return all matches sorted
/// best-first.  Candidates that do not match are excluded.
///
/// The query is matched case-insensitively.  An empty query returns all
/// candidates with equal scores.
pub fn fuzzy_match<'a>(query: &str, candidates: &[&'a str]) -> Vec<FuzzyMatch<'a>> {
    let q = query.to_lowercase();
    let mut results: Vec<FuzzyMatch<'a>> = candidates
        .iter()
        .filter_map(|&c| {
            let score = score(c, &q);
            if score >= 0 {
                Some(FuzzyMatch {
                    candidate: c,
                    score,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort best first; stable sort preserves alphabetical order within same score.
    results.sort_by(|a, b| b.score.cmp(&a.score).then(a.candidate.cmp(b.candidate)));
    results
}

/// Score a single candidate against a lowercase query.
/// Returns ≥ 0 for a match, -1 for no match.
fn score(candidate: &str, query: &str) -> i32 {
    if query.is_empty() {
        return 0;
    }

    let c_lower = candidate.to_lowercase();

    // Exact match
    if c_lower == query {
        return 1000;
    }

    // Prefix match
    if c_lower.starts_with(query) {
        return 800 + (100 - candidate.len().min(100) as i32);
    }

    // Substring match
    if let Some(pos) = c_lower.find(query) {
        let bonus = if pos == 0 { 50 } else { 0 };
        return 400 + bonus + (100 - candidate.len().min(100) as i32);
    }

    // Word-boundary subsequence: all query chars appear at separator positions.
    if word_boundary_match(&c_lower, query) {
        return 200;
    }

    // General subsequence: all query chars appear in order.
    if subsequence_match(&c_lower, query) {
        return 100;
    }

    -1
}

/// True if every character in `query` appears in `text` as a word-initial letter.
/// Word boundaries are defined as the start of `text` or a position immediately
/// following `-`, `_`, or `/`.
fn word_boundary_match(text: &str, query: &str) -> bool {
    let boundary_chars: Vec<char> = {
        let mut bc = vec![];
        let chars: Vec<char> = text.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if i == 0 || matches!(chars[i - 1], '-' | '_' | '/') {
                bc.push(c);
            }
        }
        bc
    };

    let mut qi = boundary_chars.iter();
    for qc in query.chars() {
        if qi.find(|&&bc| bc == qc).is_none() {
            return false;
        }
    }
    true
}

/// True if every character in `query` appears in `text` in order (subsequence).
fn subsequence_match(text: &str, query: &str) -> bool {
    let mut ti = text.chars();
    for qc in query.chars() {
        if ti.find(|&tc| tc == qc).is_none() {
            return false;
        }
    }
    true
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<&'static str> {
        vec![
            "pods",
            "pod",
            "deployments",
            "deploy",
            "daemonsets",
            "nodes",
            "namespaces",
            "ns",
            "services",
            "svc",
            "configmaps",
            "cm",
        ]
    }

    #[test]
    fn exact_match_ranks_first() {
        let results = fuzzy_match("pods", &candidates());
        assert_eq!(results[0].candidate, "pods");
        assert!(results[0].score >= 1000);
    }

    #[test]
    fn prefix_match_beats_substring() {
        // "po" is a prefix of "pods" and "pod" but not "deployments"
        let results = fuzzy_match("po", &candidates());
        let first_two: Vec<_> = results.iter().take(2).map(|r| r.candidate).collect();
        assert!(first_two.contains(&"pod"));
        assert!(first_two.contains(&"pods"));
    }

    #[test]
    fn empty_query_returns_all_candidates() {
        let results = fuzzy_match("", &candidates());
        assert_eq!(results.len(), candidates().len());
    }

    #[test]
    fn no_match_excluded() {
        let results = fuzzy_match("zzz", &candidates());
        assert!(results.is_empty());
    }

    #[test]
    fn case_insensitive() {
        let results = fuzzy_match("POD", &candidates());
        assert!(!results.is_empty());
        assert_eq!(results[0].candidate, "pod");
    }

    #[test]
    fn subsequence_match_works() {
        // "ds" → subsequence of "daemonsets" and exact match for nothing
        let words = vec!["daemonsets", "ds"];
        let results = fuzzy_match("ds", &words);
        // "ds" is exact, so it should rank first
        assert_eq!(results[0].candidate, "ds");
    }

    #[test]
    fn deploy_matches_deployments() {
        let results = fuzzy_match("dep", &candidates());
        let matched: Vec<_> = results.iter().map(|r| r.candidate).collect();
        assert!(matched.contains(&"deployments") || matched.contains(&"deploy"));
    }

    #[test]
    fn sorted_best_first() {
        let results = fuzzy_match("n", &candidates());
        // All have score >= 0; verify they are non-increasing by score.
        let scores: Vec<i32> = results.iter().map(|r| r.score).collect();
        for w in scores.windows(2) {
            assert!(w[0] >= w[1], "scores not sorted: {scores:?}");
        }
    }
}
