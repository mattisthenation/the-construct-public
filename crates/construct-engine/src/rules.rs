use construct_config::{Config, Rule};

/// Find the rule whose tag matches; returns the rule and its agent name.
pub fn match_rule<'a>(cfg: &'a Config, tag: &str) -> Option<&'a Rule> {
    cfg.rule_for_tag(tag)
}

/// The set of trigger tags the watcher should recognize, derived from config.
pub fn known_tags(cfg: &Config) -> Vec<String> {
    cfg.rules.iter().map(|r| r.match_tag.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let toml = r#"
[construct]
name = "C"
[vault]
path = "/v"
[[agents]]
name = "Scout"
domain = "research"
provider = "ollama"
model = "m"
base_url = "http://localhost:11434"
[[rules]]
match_tag = "theconstruct/research"
agent = "Scout"
pipeline = "research"
"#;
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn matches_known_tag() {
        let c = cfg();
        let r = match_rule(&c, "theconstruct/research").unwrap();
        assert_eq!(r.agent, "Scout");
    }

    #[test]
    fn no_match_for_unknown_tag() {
        assert!(match_rule(&cfg(), "other").is_none());
    }

    #[test]
    fn known_tags_lists_triggers() {
        assert_eq!(known_tags(&cfg()), vec!["theconstruct/research"]);
    }
}
