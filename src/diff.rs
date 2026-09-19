//! The drift-diffing logic: pure set operations over two already-parsed
//! `KEY -> value` maps (see `dotenv.rs` for how those maps get built). Kept
//! separate from parsing and from the CLI/IO layer so it's trivially unit
//! testable on hand-built maps, no temp files required.

use std::collections::BTreeMap;

/// The three-way drift classification this tool exists to report, plus
/// what matched cleanly (so "nothing wrong" isn't reported as "nothing
/// checked").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DriftReport {
    /// Documented in `.env.example`, absent from `.env` entirely. Will
    /// break at runtime — nothing provides this variable a value at all.
    pub missing: Vec<String>,
    /// Present in both files, but `.env`'s value is empty while
    /// `.env.example`'s value is non-empty — i.e. the example suggests a
    /// real value belongs here, and none was ever filled in.
    pub unfilled: Vec<String>,
    /// Present in `.env`, not documented in `.env.example` at all. Not a
    /// local breakage, but an onboarding gap for the next person.
    pub extra: Vec<String>,
    /// Present in both files and fine: either both have a real value, or
    /// both are empty (an intentionally-optional variable isn't drift).
    pub matched: Vec<String>,
}

impl DriftReport {
    pub fn has_drift(&self) -> bool {
        !self.missing.is_empty() || !self.unfilled.is_empty() || !self.extra.is_empty()
    }

    pub fn problem_count(&self) -> usize {
        self.missing.len() + self.unfilled.len() + self.extra.len()
    }
}

/// Diffs a parsed `.env.example` against a parsed `.env`, classifying every
/// key from either side into exactly one of missing/unfilled/extra/matched.
/// Keys come out sorted (both inputs are `BTreeMap`s already, so this falls
/// out for free) for deterministic, diffable CLI output.
pub fn diff(example: &BTreeMap<String, String>, env: &BTreeMap<String, String>) -> DriftReport {
    let mut report = DriftReport::default();

    for (key, example_value) in example {
        match env.get(key) {
            None => report.missing.push(key.clone()),
            Some(env_value) => {
                let needs_real_value = !example_value.trim().is_empty();
                let is_empty = env_value.trim().is_empty();
                if needs_real_value && is_empty {
                    report.unfilled.push(key.clone());
                } else {
                    report.matched.push(key.clone());
                }
            }
        }
    }

    for key in env.keys() {
        if !example.contains_key(key) {
            report.extra.push(key.clone());
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn no_drift_when_everything_matches() {
        let example = map(&[("PORT", "3000"), ("OPTIONAL_FLAG", "")]);
        let env = map(&[("PORT", "3000"), ("OPTIONAL_FLAG", "")]);
        let report = diff(&example, &env);
        assert!(!report.has_drift());
        assert_eq!(report.matched, vec!["OPTIONAL_FLAG", "PORT"]);
        assert!(report.missing.is_empty());
        assert!(report.unfilled.is_empty());
        assert!(report.extra.is_empty());
    }

    #[test]
    fn missing_when_documented_but_entirely_absent_from_env() {
        let example = map(&[("DATABASE_URL", "postgres://localhost/db")]);
        let env = map(&[]);
        let report = diff(&example, &env);
        assert_eq!(report.missing, vec!["DATABASE_URL"]);
        assert!(report.unfilled.is_empty());
        assert!(report.has_drift());
    }

    #[test]
    fn unfilled_when_present_but_empty_and_example_wants_a_real_value() {
        let example = map(&[("API_KEY", "sk-realistic-placeholder")]);
        let env = map(&[("API_KEY", "")]);
        let report = diff(&example, &env);
        assert_eq!(report.unfilled, vec!["API_KEY"]);
        assert!(report.missing.is_empty());
    }

    #[test]
    fn empty_in_both_is_matched_not_unfilled() {
        // The example itself documents this as optional (empty default) —
        // an empty .env value isn't drift in that case.
        let example = map(&[("FEATURE_FLAG", "")]);
        let env = map(&[("FEATURE_FLAG", "")]);
        let report = diff(&example, &env);
        assert_eq!(report.matched, vec!["FEATURE_FLAG"]);
        assert!(report.unfilled.is_empty());
    }

    #[test]
    fn extra_when_present_in_env_but_undocumented() {
        let example = map(&[]);
        let env = map(&[("DEBUG_MODE", "true")]);
        let report = diff(&example, &env);
        assert_eq!(report.extra, vec!["DEBUG_MODE"]);
        assert!(report.has_drift());
    }

    #[test]
    fn whitespace_only_value_counts_as_empty() {
        let example = map(&[("TOKEN", "value")]);
        let env = map(&[("TOKEN", "   ")]);
        let report = diff(&example, &env);
        assert_eq!(report.unfilled, vec!["TOKEN"]);
    }

    #[test]
    fn full_three_way_scenario_plus_one_clean_match() {
        let example = map(&[
            ("DATABASE_URL", "postgres://localhost/app"),
            ("API_KEY", "sk-placeholder"),
            ("PORT", "3000"),
        ]);
        let env = map(&[("API_KEY", ""), ("PORT", "3000"), ("DEBUG_MODE", "true")]);
        let report = diff(&example, &env);
        assert_eq!(report.missing, vec!["DATABASE_URL"]);
        assert_eq!(report.unfilled, vec!["API_KEY"]);
        assert_eq!(report.extra, vec!["DEBUG_MODE"]);
        assert_eq!(report.matched, vec!["PORT"]);
        assert_eq!(report.problem_count(), 3);
        assert!(report.has_drift());
    }
}
