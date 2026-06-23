use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Config {
    pub construct: ConstructMeta,
    pub vault: Vault,
    #[serde(default)]
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub tools: Tools,
    #[serde(default)]
    pub actions: ActionsCfg,
    #[serde(default)]
    pub inbox: Option<InboxCfg>,
    #[serde(default)]
    pub journal: Option<JournalCfg>,
    #[serde(default)]
    pub schedule: Option<ScheduleCfg>,
    #[serde(default)]
    pub briefs: Option<BriefsCfg>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ConstructMeta {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Vault {
    pub path: String,
    #[serde(default)]
    pub managed_folder: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Agent {
    pub name: String,
    pub domain: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub system_prompt_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Rule {
    pub match_tag: String,
    pub agent: String,
    pub pipeline: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Tools {
    #[serde(default)]
    pub web_search: Option<WebSearchCfg>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct WebSearchCfg {
    pub backend: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct ActionsCfg {
    #[serde(default)]
    pub tag: TagActionCfg,
    #[serde(default)]
    pub organize: OrganizeActionCfg,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TagActionCfg {
    #[serde(default = "default_max_tags")]
    pub max_tags: usize,
}
impl Default for TagActionCfg {
    fn default() -> Self {
        TagActionCfg {
            max_tags: default_max_tags(),
        }
    }
}
fn default_max_tags() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct OrganizeActionCfg {
    #[serde(default)]
    pub exclude_dirs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct InboxCfg {
    #[serde(default = "default_inbox_folder")]
    pub folder: String,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u64,
    #[serde(default)]
    pub agent: Option<String>,
}
fn default_inbox_folder() -> String {
    "Inbox".to_string()
}
fn default_idle_minutes() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct JournalCfg {
    #[serde(default = "default_journal_folder")]
    pub folder: String,
}
fn default_journal_folder() -> String {
    "journal".to_string()
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ScheduleCfg {
    #[serde(default = "default_daily_time")]
    pub daily_time: String,
}
fn default_daily_time() -> String {
    "01:00".to_string()
}

/// Watch a vault folder of externally-written Daily Briefs and fold each
/// day's brief into that day's journal note.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BriefsCfg {
    #[serde(default = "default_briefs_folder")]
    pub folder: String,
}
fn default_briefs_folder() -> String {
    "AI/DailyBriefs".to_string()
}

/// The set of built-in pipeline names this binary knows how to run. The three
/// spec handler names (`remind-me`, `file-this`, `research-this`) are canonical;
/// the older internal names remain as accepted aliases.
pub const KNOWN_PIPELINES: &[&str] = &[
    "remind-me",
    "file-this",
    "research-this",
    "research",
    "summarize",
    "tag",
    "organize",
];

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config: {0}")]
    Io(String),
    #[error("could not parse config: {0}")]
    Parse(String),
    #[error("invalid config: {0}")]
    Validation(String),
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?;
        let cfg: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Every rule must reference a defined agent; every agent tool must be known.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for rule in &self.rules {
            if !self.agents.iter().any(|a| a.name == rule.agent) {
                return Err(ConfigError::Validation(format!(
                    "rule for tag '{}' references unknown agent '{}'",
                    rule.match_tag, rule.agent
                )));
            }
            if !KNOWN_PIPELINES.contains(&rule.pipeline.as_str()) {
                return Err(ConfigError::Validation(format!(
                    "rule for tag '{}' names unknown pipeline '{}' (known: {:?})",
                    rule.match_tag, rule.pipeline, KNOWN_PIPELINES
                )));
            }
        }
        if let Some(inbox) = &self.inbox {
            if inbox.idle_minutes == 0 {
                return Err(ConfigError::Validation(
                    "inbox.idle_minutes must be greater than 0".into(),
                ));
            }
            if let Some(agent) = &inbox.agent {
                if !self.agents.iter().any(|a| &a.name == agent) {
                    return Err(ConfigError::Validation(format!(
                        "inbox.agent references unknown agent '{agent}'"
                    )));
                }
            }
        }
        if let Some(schedule) = &self.schedule {
            if !is_valid_hhmm(&schedule.daily_time) {
                return Err(ConfigError::Validation(format!(
                    "schedule.daily_time '{}' is not valid HH:MM (24h)",
                    schedule.daily_time
                )));
            }
        }
        if let Some(briefs) = &self.briefs {
            if briefs.folder.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "briefs.folder must not be empty".into(),
                ));
            }
        }
        if let Some(ws) = &self.tools.web_search {
            // `api_key_env` is the NAME of an environment variable, not the key. A
            // valid env-var name is [A-Za-z_][A-Za-z0-9_]*; a pasted key (e.g.
            // "tvly-…") contains dashes and fails this — catch that common foot-gun.
            if !is_valid_env_var_name(&ws.api_key_env) {
                return Err(ConfigError::Validation(format!(
                    "tools.web_search.api_key_env '{}' is not a valid environment variable \
                     name — it must be the NAME of an env var (e.g. TAVILY_API_KEY) that \
                     holds the key, not the key itself",
                    ws.api_key_env
                )));
            }
        }
        Ok(())
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.name == name)
    }

    pub fn rule_for_tag(&self, tag: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.match_tag == tag)
    }
}

/// True if `s` is a valid POSIX-ish environment variable name: a non-empty string of
/// `[A-Za-z0-9_]` not starting with a digit. (Rejects a pasted API key, which has dashes.)
fn is_valid_env_var_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// True if `s` is `H:MM` or `HH:MM` 24-hour time. Minute must be two digits.
fn is_valid_hhmm(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 || parts[1].len() != 2 {
        return false;
    }
    let (Ok(h), Ok(m)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) else {
        return false;
    };
    h <= 23 && m <= 59
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample() -> &'static str {
        r#"
[construct]
name = "The Construct"

[vault]
path = "/tmp/vault"
managed_folder = "Construct"

[[agents]]
name = "Scout"
domain = "research"
provider = "ollama"
model = "qwen2.5:14b"
base_url = "http://localhost:11434"
tools = ["web_search", "web_fetch"]

[tools.web_search]
backend = "tavily"
api_key_env = "TAVILY_API_KEY"

[[rules]]
match_tag = "theconstruct/research"
agent = "Scout"
pipeline = "research"
"#
    }

    #[test]
    fn rejects_key_shaped_api_key_env() {
        // A pasted Tavily key (with dashes) is not a valid env-var name → rejected.
        let bad = sample().replace(
            "api_key_env = \"TAVILY_API_KEY\"",
            "api_key_env = \"tvly-dev-abc123\"",
        );
        let cfg: Config = toml::from_str(&bad).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
        // The conventional NAME form validates fine.
        let cfg: Config = toml::from_str(sample()).unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_and_validates_sample() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.construct.name, "The Construct");
        assert_eq!(cfg.agent("Scout").unwrap().model, "qwen2.5:14b");
        assert_eq!(
            cfg.rule_for_tag("theconstruct/research").unwrap().agent,
            "Scout"
        );
    }

    #[test]
    fn rejects_rule_with_unknown_agent() {
        let bad = sample().replace("agent = \"Scout\"\npipeline", "agent = \"Ghost\"\npipeline");
        let cfg: Config = toml::from_str(&bad).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_unknown_pipeline() {
        let toml = r#"
[construct]
name = "C"
[vault]
path = "/v"
[[agents]]
name = "A"
domain = "d"
provider = "ollama"
model = "m"
base_url = "http://localhost:11434"
[[rules]]
match_tag = "theconstruct/bogus"
agent = "A"
pipeline = "bogus"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn accepts_known_pipelines_and_actions_defaults() {
        let toml = r#"
[construct]
name = "C"
[vault]
path = "/v"
[[agents]]
name = "Lib"
domain = "notes"
provider = "ollama"
model = "m"
base_url = "http://localhost:11434"
[[rules]]
match_tag = "theconstruct/tag"
agent = "Lib"
pipeline = "tag"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.actions.tag.max_tags, 8); // default
    }

    #[test]
    fn load_from_file() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(sample().as_bytes()).unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.vault.path, "/tmp/vault");
    }

    #[test]
    fn features_off_when_tables_absent() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        assert!(cfg.inbox.is_none());
        assert!(cfg.journal.is_none());
        assert!(cfg.schedule.is_none());
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_inbox_journal_schedule() {
        let toml = format!(
            "{}\n[inbox]\nfolder = \"Inbox\"\nidle_minutes = 45\n\n[journal]\nfolder = \"journal\"\n\n[schedule]\ndaily_time = \"01:00\"\n",
            sample()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let inbox = cfg.inbox.unwrap();
        assert_eq!(inbox.folder, "Inbox");
        assert_eq!(inbox.idle_minutes, 45);
        assert_eq!(cfg.journal.unwrap().folder, "journal");
        assert_eq!(cfg.schedule.unwrap().daily_time, "01:00");
    }

    #[test]
    fn inbox_defaults_folder_and_idle() {
        let toml = format!("{}\n[inbox]\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        let inbox = cfg.inbox.unwrap();
        assert_eq!(inbox.folder, "Inbox");
        assert_eq!(inbox.idle_minutes, 30);
    }

    #[test]
    fn rejects_zero_idle_minutes() {
        let toml = format!("{}\n[inbox]\nidle_minutes = 0\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn rejects_bad_daily_time() {
        for bad in ["25:00", "01:60", "noon", "0100"] {
            let toml = format!("{}\n[schedule]\ndaily_time = \"{}\"\n", sample(), bad);
            let cfg: Config = toml::from_str(&toml).unwrap();
            assert!(
                matches!(cfg.validate(), Err(ConfigError::Validation(_))),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn accepts_good_daily_times() {
        for ok in ["00:00", "01:00", "23:59", "9:05"] {
            let toml = format!("{}\n[schedule]\ndaily_time = \"{}\"\n", sample(), ok);
            let cfg: Config = toml::from_str(&toml).unwrap();
            assert!(cfg.validate().is_ok(), "should accept {ok}");
        }
    }

    #[test]
    fn inbox_agent_defaults_to_none() {
        let toml = format!("{}\n[inbox]\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(cfg.inbox.unwrap().agent.is_none());
    }

    #[test]
    fn inbox_agent_must_exist_when_named() {
        // Scout IS defined in sample(); Ghost is not.
        let good = format!("{}\n[inbox]\nagent = \"Scout\"\n", sample());
        toml::from_str::<Config>(&good).unwrap().validate().unwrap();

        let bad = format!("{}\n[inbox]\nagent = \"Ghost\"\n", sample());
        let cfg: Config = toml::from_str(&bad).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn briefs_off_when_table_absent() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        assert!(cfg.briefs.is_none());
    }

    #[test]
    fn briefs_defaults_folder() {
        let toml = format!("{}\n[briefs]\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.briefs.unwrap().folder, "AI/DailyBriefs");
    }

    #[test]
    fn rejects_empty_briefs_folder() {
        let toml = format!("{}\n[briefs]\nfolder = \"\"\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }
}
