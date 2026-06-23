/// A parsed Obsidian note: optional YAML frontmatter + body.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub frontmatter: serde_yaml::Mapping,
    pub body: String,
}

impl Note {
    /// Parse a markdown string into frontmatter + body.
    pub fn parse(text: &str) -> Note {
        if let Some(rest) = text.strip_prefix("---\n") {
            if let Some(end) = rest.find("\n---\n") {
                let yaml = &rest[..end];
                let body = &rest[end + 5..];
                if let Ok(serde_yaml::Value::Mapping(m)) = serde_yaml::from_str(yaml) {
                    return Note {
                        frontmatter: m,
                        body: body.to_string(),
                    };
                }
            }
        }
        Note {
            frontmatter: serde_yaml::Mapping::new(),
            body: text.to_string(),
        }
    }

    /// Serialize back to a markdown string (frontmatter only emitted if non-empty).
    // Renders to the on-disk note text; kept as an inherent `to_string` so existing
    // `note.to_string()` callers are unchanged.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        if self.frontmatter.is_empty() {
            return self.body.clone();
        }
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(self.frontmatter.clone()))
            .unwrap_or_default();
        format!("---\n{}---\n{}", yaml, self.body)
    }

    /// Read a string value from frontmatter.
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.frontmatter
            .get(serde_yaml::Value::from(key))
            .and_then(|v| v.as_str().map(String::from))
    }

    /// Set a string value in frontmatter.
    pub fn set_str(&mut self, key: &str, value: &str) {
        self.frontmatter
            .insert(serde_yaml::Value::from(key), serde_yaml::Value::from(value));
    }

    pub fn remove(&mut self, key: &str) {
        self.frontmatter.remove(serde_yaml::Value::from(key));
    }

    /// Merge tags into the frontmatter `tags:` sequence (union, no duplicates,
    /// preserving existing). Creates the key if absent.
    pub fn merge_tags(&mut self, new_tags: &[String]) {
        use serde_yaml::Value;
        let key = Value::from("tags");
        let mut existing: Vec<String> = match self.frontmatter.get(&key) {
            Some(Value::Sequence(seq)) => seq
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => Vec::new(),
        };
        for t in new_tags {
            if !existing.iter().any(|e| e == t) {
                existing.push(t.clone());
            }
        }
        let seq: Vec<Value> = existing.into_iter().map(Value::from).collect();
        self.frontmatter.insert(key, Value::Sequence(seq));
    }

    /// Collect Obsidian inline tags (#a/b) from the body.
    pub fn tags(&self) -> Vec<String> {
        let mut tags = vec![];
        for token in self.body.split_whitespace() {
            if let Some(t) = token.strip_prefix('#') {
                let clean: String = t
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '/' || *c == '_' || *c == '-')
                    .collect();
                if !clean.is_empty() {
                    tags.push(clean);
                }
            }
        }
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\ntitle: Hi\n---\nbody #theconstruct/research here";
        let note = Note::parse(text);
        assert_eq!(note.get_str("title").as_deref(), Some("Hi"));
        assert!(note.body.contains("body"));
        assert_eq!(note.tags(), vec!["theconstruct/research"]);
    }

    #[test]
    fn handles_no_frontmatter() {
        let note = Note::parse("just body");
        assert!(note.frontmatter.is_empty());
        assert_eq!(note.body, "just body");
    }

    #[test]
    fn set_and_round_trip() {
        let mut note = Note::parse("body");
        note.set_str("construct_status", "queued");
        let s = note.to_string();
        let back = Note::parse(&s);
        assert_eq!(back.get_str("construct_status").as_deref(), Some("queued"));
        assert_eq!(back.body, "body");
    }

    #[test]
    fn remove_key() {
        let mut note = Note::parse("---\na: b\n---\nx");
        note.remove("a");
        assert!(note.get_str("a").is_none());
    }

    #[test]
    fn merge_tags_unions_and_dedupes() {
        let mut note = Note::parse("---\ntags:\n- rust\n---\nbody");
        note.merge_tags(&["rust".into(), "cli".into()]);
        let out = note.to_string();
        let back = Note::parse(&out);
        let tags = back
            .frontmatter
            .get(serde_yaml::Value::from("tags"))
            .unwrap();
        let seq = tags.as_sequence().unwrap();
        let vals: Vec<&str> = seq.iter().filter_map(|v| v.as_str()).collect();
        assert!(vals.contains(&"rust"));
        assert!(vals.contains(&"cli"));
        assert_eq!(vals.iter().filter(|t| **t == "rust").count(), 1);
    }
}
