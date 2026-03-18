use anyhow::{Context, Result};

use super::ruleset::Ruleset;
use crate::audit::hash_hex;

/// Parse the embedded ruleset JSON into a Ruleset struct.
pub fn load_embedded_ruleset() -> Result<Ruleset> {
    let source = include_str!("../../assets/ruleset.json");
    serde_json::from_str(source).context("parse embedded ruleset JSON")
}

/// Compute a combined hash of all rules (spelling + case) for reproducibility tracking.
/// This hash changes whenever base rules or overrides change.
pub fn compute_ruleset_hash(
    spelling_rules: &[super::ruleset::SpellingRule],
    case_rules: &[super::ruleset::CaseRule],
) -> String {
    let canonical = serde_json::json!({
        "spelling": spelling_rules,
        "case": case_rules,
    });
    let bytes = serde_json::to_vec(&canonical).expect("Value serialization is infallible");
    hash_hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::ruleset::{CaseRule, RuleType, SpellingRule};

    #[test]
    fn hash_deterministic() {
        let rules = vec![SpellingRule {
            from: "軟件".into(),
            to: vec!["軟體".into()],
            rule_type: RuleType::CrossStrait,

            disabled: false,
            context: None,
            english: None,
            exceptions: None,
            context_clues: None,
            negative_context_clues: None,
            tags: None,
        }];
        let case_rules = vec![CaseRule {
            term: "JavaScript".into(),
            alternatives: None,
            disabled: false,
        }];

        let h1 = compute_ruleset_hash(&rules, &case_rules);
        let h2 = compute_ruleset_hash(&rules, &case_rules);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn hash_changes_with_rules() {
        let rules_a = vec![SpellingRule {
            from: "軟件".into(),
            to: vec!["軟體".into()],
            rule_type: RuleType::CrossStrait,

            disabled: false,
            context: None,
            english: None,
            exceptions: None,
            context_clues: None,
            negative_context_clues: None,
            tags: None,
        }];
        let rules_b = vec![SpellingRule {
            from: "內存".into(),
            to: vec!["記憶體".into()],
            rule_type: RuleType::CrossStrait,

            disabled: false,
            context: None,
            english: None,
            exceptions: None,
            context_clues: None,
            negative_context_clues: None,
            tags: None,
        }];
        let case_rules: Vec<CaseRule> = vec![];

        let h1 = compute_ruleset_hash(&rules_a, &case_rules);
        let h2 = compute_ruleset_hash(&rules_b, &case_rules);
        assert_ne!(h1, h2);
    }

    #[test]
    fn embedded_ruleset_parses() {
        let source = include_str!("../../assets/ruleset.json");
        let ruleset: Ruleset = serde_json::from_str(source).unwrap();
        assert!(!ruleset.spelling_rules.is_empty());
        assert!(!ruleset.case_rules.is_empty());
    }
}
