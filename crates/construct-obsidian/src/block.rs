#[cfg(test)]
const START: &str = "<!-- construct:research:start -->";
#[cfg(test)]
const END: &str = "<!-- construct:research:end -->";

fn markers(name: &str) -> (String, String) {
    (
        format!("<!-- construct:{name}:start -->"),
        format!("<!-- construct:{name}:end -->"),
    )
}

/// Insert or replace a named managed block at the END of the body.
pub fn upsert_named(body: &str, name: &str, content: &str) -> String {
    let (start, end) = markers(name);
    let block = format!("{start}\n{content}\n{end}");
    if let (Some(s), Some(e)) = (body.find(&start), body.find(&end)) {
        let mut out = String::new();
        out.push_str(&body[..s]);
        out.push_str(&block);
        out.push_str(&body[e + end.len()..]);
        out
    } else {
        let sep = if body.ends_with('\n') || body.is_empty() {
            ""
        } else {
            "\n"
        };
        format!("{body}{sep}\n{block}\n")
    }
}

/// Insert or replace a named managed block at the TOP of the body.
pub fn upsert_named_at_top(body: &str, name: &str, content: &str) -> String {
    let (start, end) = markers(name);
    let block = format!("{start}\n{content}\n{end}");
    if let (Some(s), Some(e)) = (body.find(&start), body.find(&end)) {
        // Replace in place.
        let mut out = String::new();
        out.push_str(&body[..s]);
        out.push_str(&block);
        out.push_str(&body[e + end.len()..]);
        out
    } else {
        let sep = if body.is_empty() { "" } else { "\n\n" };
        format!("{block}{sep}{body}")
    }
}

/// Remove a named managed block entirely.
pub fn remove_named(body: &str, name: &str) -> String {
    let (start, end) = markers(name);
    if let (Some(s), Some(e)) = (body.find(&start), body.find(&end)) {
        let mut out = String::new();
        out.push_str(body[..s].trim_end());
        out.push_str(&body[e + end.len()..]);
        out
    } else {
        body.to_string()
    }
}

/// Read the inner content of a named managed block, if present.
/// Returns the text between the start and end markers (trimming the single
/// newline that `upsert_named` inserts on each side).
pub fn read_named(body: &str, name: &str) -> Option<String> {
    let (start, end) = markers(name);
    let s = body.find(&start)? + start.len();
    let e = body.find(&end)?;
    if e < s {
        return None;
    }
    Some(body[s..e].trim_matches('\n').to_string())
}

/// Back-compat: research block at end of body.
pub fn upsert_block(body: &str, content: &str) -> String {
    upsert_named(body, "research", content)
}

/// Back-compat: remove the research block.
pub fn remove_block(body: &str) -> String {
    remove_named(body, "research")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_when_absent() {
        let out = upsert_block("hello", "RESULT");
        assert!(out.contains(START));
        assert!(out.contains("RESULT"));
        assert!(out.contains(END));
        assert!(out.starts_with("hello"));
    }

    #[test]
    fn replaces_when_present() {
        let first = upsert_block("hello", "OLD");
        let second = upsert_block(&first, "NEW");
        assert!(second.contains("NEW"));
        assert!(!second.contains("OLD"));
        // exactly one block
        assert_eq!(second.matches(START).count(), 1);
    }

    #[test]
    fn removes_block() {
        let with = upsert_block("hello", "X");
        let without = remove_block(&with);
        assert!(!without.contains(START));
        assert!(without.contains("hello"));
    }

    #[test]
    fn named_blocks_are_independent() {
        let b = upsert_named("body", "summary", "SUM");
        let b = upsert_named(&b, "research", "RES");
        assert!(b.contains("construct:summary:start"));
        assert!(b.contains("construct:research:start"));
        assert!(b.contains("SUM"));
        assert!(b.contains("RES"));
        // replacing summary leaves research intact
        let b2 = upsert_named(&b, "summary", "SUM2");
        assert!(b2.contains("SUM2"));
        assert!(!b2.contains("SUM\n"));
        assert!(b2.contains("RES"));
    }

    #[test]
    fn upsert_at_top_places_block_first() {
        let out = upsert_named_at_top("hello body", "summary", "TLDR");
        assert!(out
            .trim_start()
            .starts_with("<!-- construct:summary:start -->"));
        assert!(out.contains("hello body"));
    }

    #[test]
    fn research_back_compat() {
        let out = upsert_block("hello", "RESULT");
        assert!(out.contains("construct:research:start"));
        assert!(out.contains("RESULT"));
    }

    #[test]
    fn read_named_returns_inner_content() {
        let b = upsert_named("body", "inbox-log", "line one\nline two");
        assert_eq!(
            read_named(&b, "inbox-log").as_deref(),
            Some("line one\nline two")
        );
        assert_eq!(read_named(&b, "absent"), None);
        assert_eq!(read_named("no blocks here", "inbox-log"), None);
    }
}
