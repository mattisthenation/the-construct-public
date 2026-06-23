use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SummaryOut {
    pub tldr: String,
    #[serde(default)]
    pub action_items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TagsOut {
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OrganizeOut {
    pub destination: String,
    #[serde(default)]
    pub reason: String,
}
