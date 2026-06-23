use crate::actions::{OrganizeOut, SummaryOut, TagsOut};
use construct_core::types::ResearchResult;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GateError {
    #[error("agent output was not valid JSON: {0}")]
    NotJson(String),
    #[error("agent output failed validation: {0}")]
    Invalid(String),
    #[error("agent cited an ungrounded source not found in gathered evidence: {0}")]
    Ungrounded(String),
}

/// Extract a JSON object from the model's free text (it may wrap JSON in prose
/// or a ```json fence), validate its shape, then verify every cited source URL
/// is grounded in `evidence` (the tool outputs + fetched URLs the agent actually
/// gathered). This rejects fabricated sources from an unreliable local model.
pub fn validate(raw: &str, evidence: &str) -> Result<ResearchResult, GateError> {
    let json_slice =
        extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let result: ResearchResult =
        serde_json::from_str(json_slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    if result.summary.trim().is_empty() {
        return Err(GateError::Invalid("summary is empty".into()));
    }
    if result.findings.is_empty() {
        return Err(GateError::Invalid("findings is empty".into()));
    }
    if result.sources.is_empty() {
        return Err(GateError::Invalid("no sources cited".into()));
    }
    for s in &result.sources {
        if s.url.trim().is_empty() || !evidence.contains(s.url.trim()) {
            return Err(GateError::Ungrounded(s.url.clone()));
        }
    }
    Ok(result)
}

/// Validate a summarize action's output: valid JSON, non-empty tldr.
pub fn validate_summary(raw: &str) -> Result<SummaryOut, GateError> {
    let slice =
        extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: SummaryOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    if out.tldr.trim().is_empty() {
        return Err(GateError::Invalid("tldr is empty".into()));
    }
    Ok(out)
}

/// Normalize a single tag: lowercase, strip leading '#', spaces→'-', trim.
fn normalize_tag(t: &str) -> String {
    t.trim()
        .trim_start_matches('#')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

/// Validate a tag action's output: valid JSON, normalize, dedupe, cap.
pub fn validate_tags(raw: &str, max_tags: usize) -> Result<Vec<String>, GateError> {
    let slice =
        extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: TagsOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for t in out.tags {
        let n = normalize_tag(&t);
        if n.is_empty() || !seen.insert(n.clone()) {
            continue;
        }
        result.push(n);
        if result.len() >= max_tags {
            break;
        }
    }
    if result.is_empty() {
        return Err(GateError::Invalid("no usable tags".into()));
    }
    Ok(result)
}

/// Validate an organize action: valid JSON; destination must be one of `folders`.
pub fn validate_organize(raw: &str, folders: &[String]) -> Result<OrganizeOut, GateError> {
    let slice =
        extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: OrganizeOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    let dest = out.destination.trim().trim_matches('/');
    if dest.is_empty() || !folders.iter().any(|f| f == dest) {
        return Err(GateError::Invalid(format!(
            "destination '{}' is not an existing vault folder",
            out.destination
        )));
    }
    Ok(OrganizeOut {
        destination: dest.to_string(),
        reason: out.reason,
    })
}

/// Validate an Inbox move suggestion: valid JSON, non-empty destination.
/// Unlike `validate_organize`, the destination is NOT required to be an existing
/// folder — an unknown destination is a recommendation for the human, not an error.
/// The caller decides whether to auto-move (destination is an existing folder) or
/// merely recommend (destination does not yet exist).
pub fn validate_destination(raw: &str) -> Result<OrganizeOut, GateError> {
    let slice =
        extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: OrganizeOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    let dest = out.destination.trim().trim_matches('/').trim();
    if dest.is_empty() {
        return Err(GateError::Invalid("destination is empty".into()));
    }
    Ok(OrganizeOut {
        destination: dest.to_string(),
        reason: out.reason,
    })
}

/// Find the first balanced top-level {...} span. Pure + testable.
fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Structured output for the daily-recap agent.
#[derive(Debug, serde::Deserialize)]
pub struct Recap {
    pub tldr: String,
    #[serde(default)]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<String>,
}

/// Validate the daily-recap agent output: strict JSON with a non-empty tldr;
/// highlights/action_items optional. Same fence tolerance as validate_summary.
pub fn validate_recap(content: &str) -> Result<Recap, GateError> {
    let json =
        extract_json(content).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let recap: Recap = serde_json::from_str(json).map_err(|e| GateError::NotJson(e.to_string()))?;
    if recap.tldr.trim().is_empty() {
        return Err(GateError::Invalid("tldr must be non-empty".into()));
    }
    Ok(recap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_grounded_result() {
        let raw = r#"Here you go:
        ```json
        {"summary":"S","findings":["a","b"],"sources":[{"title":"t","url":"https://example.com/x"}]}
        ```"#;
        let evidence = "search results ... https://example.com/x ... more";
        let r = validate(raw, evidence).unwrap();
        assert_eq!(r.summary, "S");
        assert_eq!(r.findings.len(), 2);
    }

    #[test]
    fn rejects_missing_findings() {
        let raw = r#"{"summary":"S","findings":[],"sources":[{"title":"t","url":"u"}]}"#;
        assert_eq!(
            validate(raw, "u"),
            Err(GateError::Invalid("findings is empty".into()))
        );
    }

    #[test]
    fn rejects_no_sources() {
        let raw = r#"{"summary":"S","findings":["a"],"sources":[]}"#;
        assert_eq!(
            validate(raw, "anything"),
            Err(GateError::Invalid("no sources cited".into()))
        );
    }

    #[test]
    fn rejects_ungrounded_source() {
        // The cited URL never appeared in the evidence → fabricated.
        let raw = r#"{"summary":"S","findings":["a"],"sources":[{"title":"t","url":"https://made-up.example/page"}]}"#;
        let evidence = "search results about something else entirely";
        assert_eq!(
            validate(raw, evidence),
            Err(GateError::Ungrounded("https://made-up.example/page".into()))
        );
    }

    #[test]
    fn rejects_non_json() {
        assert!(matches!(
            validate("no json here", ""),
            Err(GateError::NotJson(_))
        ));
    }

    #[test]
    fn summary_gate_accepts_and_rejects() {
        let ok = r#"{"tldr":"Short summary","action_items":["do x"]}"#;
        let s = validate_summary(ok).unwrap();
        assert_eq!(s.tldr, "Short summary");
        assert_eq!(s.action_items.len(), 1);

        let empty = r#"{"tldr":"   ","action_items":[]}"#;
        assert!(matches!(
            validate_summary(empty),
            Err(GateError::Invalid(_))
        ));
        assert!(matches!(
            validate_summary("nope"),
            Err(GateError::NotJson(_))
        ));
    }

    #[test]
    fn tag_gate_normalizes_caps_dedupes() {
        let raw = r##"{"tags":["#Rust"," Web Dev ","rust","a","b","c","d","e","f","g"]}"##;
        let tags = validate_tags(raw, 8).unwrap();
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"web-dev".to_string()));
        assert_eq!(tags.iter().filter(|t| **t == "rust").count(), 1);
        assert!(tags.len() <= 8);
        assert!(matches!(
            validate_tags("nope", 8),
            Err(GateError::NotJson(_))
        ));
        assert!(matches!(
            validate_tags(r#"{"tags":[]}"#, 8),
            Err(GateError::Invalid(_))
        ));
    }

    #[test]
    fn extract_json_skips_braces_inside_strings() {
        // Braces inside string values must not confuse the scanner.
        let raw = r#"prose {"tldr":"use {} or } inside","action_items":[]} trailing"#;
        let s = validate_summary(raw).unwrap();
        assert_eq!(s.tldr, "use {} or } inside");
    }

    #[test]
    fn extract_json_respects_escaped_quotes() {
        let raw = r#"{"tldr":"a \"quoted\" } brace","action_items":[]}"#;
        let s = validate_summary(raw).unwrap();
        assert_eq!(s.tldr, "a \"quoted\" } brace");
    }

    #[test]
    fn validate_destination_accepts_any_nonempty_dest() {
        // No folder list: any non-empty destination is accepted (it may be a new-folder suggestion).
        let ok = r#"{"destination":"Reading/Articles","reason":"it's an article"}"#;
        let o = validate_destination(ok).unwrap();
        assert_eq!(o.destination, "Reading/Articles");
        assert_eq!(o.reason, "it's an article");

        // Trims surrounding slashes/space like validate_organize does.
        let trimmed = r#"{"destination":" /Projects/ ","reason":"x"}"#;
        assert_eq!(
            validate_destination(trimmed).unwrap().destination,
            "Projects"
        );

        // Empty destination is rejected.
        assert!(matches!(
            validate_destination(r#"{"destination":"  ","reason":"x"}"#),
            Err(GateError::Invalid(_))
        ));
        // Non-JSON rejected.
        assert!(matches!(
            validate_destination("nope"),
            Err(GateError::NotJson(_))
        ));
    }

    #[test]
    fn validate_recap_accepts_full_shape() {
        let json =
            r#"{"tldr": "Busy day.", "highlights": ["Shipped X"], "action_items": ["Email Bob"]}"#;
        let r = validate_recap(json).unwrap();
        assert_eq!(r.tldr, "Busy day.");
        assert_eq!(r.highlights, vec!["Shipped X"]);
        assert_eq!(r.action_items, vec!["Email Bob"]);
    }

    #[test]
    fn validate_recap_defaults_missing_lists() {
        let r = validate_recap(r#"{"tldr": "Quiet."}"#).unwrap();
        assert!(r.highlights.is_empty());
        assert!(r.action_items.is_empty());
    }

    #[test]
    fn validate_recap_rejects_empty_tldr_and_garbage() {
        assert!(validate_recap(r#"{"tldr": ""}"#).is_err());
        assert!(validate_recap("not json").is_err());
    }

    #[test]
    fn validate_recap_tolerates_code_fences() {
        // Mirror whatever fence-stripping validate_summary does — reuse the
        // same pre-processing helper so behavior stays consistent.
        let fenced = "```json\n{\"tldr\": \"ok\"}\n```";
        assert!(validate_recap(fenced).is_ok());
    }

    #[test]
    fn organize_gate_requires_known_destination() {
        let folders = vec!["Projects".to_string(), "Archive".to_string()];
        let ok = r#"{"destination":"Projects","reason":"active work"}"#;
        let o = validate_organize(ok, &folders).unwrap();
        assert_eq!(o.destination, "Projects");

        let bad = r#"{"destination":"Nonexistent","reason":"x"}"#;
        assert!(matches!(
            validate_organize(bad, &folders),
            Err(GateError::Invalid(_))
        ));
        assert!(matches!(
            validate_organize("nope", &folders),
            Err(GateError::NotJson(_))
        ));
    }
}
