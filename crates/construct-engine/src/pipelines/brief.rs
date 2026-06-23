//! Daily Brief pure helpers: outline extraction, day-note section rendering,
//! and the content hash used to guard re-processing.
use sha2::{Digest, Sha256};

/// Hex SHA-256 of brief content. Stable across runs/platforms (unlike the
/// std hasher), so the guard survives restarts.
pub fn content_hash(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

/// Deterministic outline of a brief: `##+` headings (bolded) and top-level
/// `-`/`*` bullets, in document order, capped at `max_items`. The top-level
/// `#` title is skipped (it duplicates the section heading we render).
pub fn extract_outline(text: &str, max_items: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= max_items {
            break;
        }
        let t = line.trim_end();
        if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("### ")) {
            out.push(format!("**{}**", h.trim()));
        } else if (t.starts_with("- ") || t.starts_with("* ")) && !t.starts_with("- [") {
            out.push(format!("- {}", t[2..].trim()));
        }
    }
    out
}

/// Render the `daily-brief` managed-block body: heading, wikilink to the brief
/// note, and its outline.
pub fn render_brief_section(brief_stem: &str, outline: &[String]) -> String {
    let mut s = format!("## Daily Brief\n\n[[{brief_stem}]]\n");
    if !outline.is_empty() {
        s.push('\n');
        for item in outline {
            s.push_str(item);
            s.push('\n');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIEF: &str = "# Daily Brief\n\nIntro sentence.\n\n## Calendar\n- 10:00 Standup\n- 14:00 1:1 with Ana\n\n## Email highlights\n- Contract signed\nSome prose here.\n- Follow up with vendor\n";

    #[test]
    fn outline_collects_headings_and_bullets_in_order() {
        let items = extract_outline(BRIEF, 10);
        assert_eq!(
            items,
            vec![
                "**Calendar**",
                "- 10:00 Standup",
                "- 14:00 1:1 with Ana",
                "**Email highlights**",
                "- Contract signed",
                "- Follow up with vendor",
            ]
        );
    }

    #[test]
    fn outline_caps_items_and_skips_top_level_title() {
        let items = extract_outline(BRIEF, 3);
        assert_eq!(items.len(), 3);
        assert!(!items.iter().any(|i| i.contains("Daily Brief")));
    }

    #[test]
    fn renders_section_with_wikilink_and_outline() {
        let s = render_brief_section(
            "2026-06-09",
            &["**Calendar**".into(), "- 10:00 Standup".into()],
        );
        assert!(s.starts_with("## Daily Brief\n"));
        assert!(s.contains("[[2026-06-09]]"));
        assert!(s.contains("- 10:00 Standup"));
    }

    #[test]
    fn renders_link_only_when_outline_empty() {
        let s = render_brief_section("2026-06-09", &[]);
        assert!(s.contains("[[2026-06-09]]"));
    }

    #[test]
    fn content_hash_is_stable_and_distinguishes() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("b"));
    }
}
