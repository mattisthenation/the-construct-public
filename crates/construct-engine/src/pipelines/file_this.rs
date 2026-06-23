//! `file-this` deterministic tier: classify a note to a destination folder using
//! plain keyword rules — no model. Routing, not reasoning. Only when no rule
//! matches does the orchestrator escalate to the model (the organize flow).

use construct_config::FileRule;

/// First rule with any keyword present in the note body (case-insensitive) wins.
/// Returns the destination folder and the keyword that matched (for the audit note).
pub fn classify<'a>(body: &str, rules: &'a [FileRule]) -> Option<(&'a str, &'a str)> {
    let hay = body.to_lowercase();
    for rule in rules {
        for kw in &rule.any_of {
            if !kw.is_empty() && hay.contains(&kw.to_lowercase()) {
                return Some((rule.folder.as_str(), kw.as_str()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<FileRule> {
        vec![
            FileRule {
                any_of: vec!["kubernetes".into(), "k8s".into()],
                folder: "DevOps".into(),
            },
            FileRule {
                any_of: vec!["invoice".into(), "receipt".into()],
                folder: "Finance".into(),
            },
        ]
    }

    #[test]
    fn matches_first_rule_by_keyword() {
        let r = rules();
        let (folder, kw) = classify("notes on k8s ingress", &r).unwrap();
        assert_eq!(folder, "DevOps");
        assert_eq!(kw, "k8s");
    }

    #[test]
    fn case_insensitive() {
        let r = rules();
        let (folder, _) = classify("Quarterly INVOICE attached", &r).unwrap();
        assert_eq!(folder, "Finance");
    }

    #[test]
    fn no_match_returns_none() {
        let r = rules();
        assert!(classify("a note about gardening", &r).is_none());
        assert!(classify("anything", &[]).is_none());
    }
}
