use async_trait::async_trait;
use construct_core::tool::{Tool, ToolError, ToolSpec};
use serde_json::{json, Value};

/// Tavily-backed web search. SearXNG can be added as a sibling impl later.
pub struct WebSearch {
    api_key: String,
    http: reqwest::Client,
    endpoint: String,
}

impl WebSearch {
    pub fn tavily(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        WebSearch {
            api_key: api_key.into(),
            http,
            endpoint: "https://api.tavily.com/search".into(),
        }
    }

    /// Pure: turn a Tavily JSON response into a compact text block for the model.
    pub fn format_results(v: &Value) -> String {
        let mut out = String::new();
        if let Some(arr) = v["results"].as_array() {
            for (i, r) in arr.iter().enumerate() {
                let title = r["title"].as_str().unwrap_or("");
                let url = r["url"].as_str().unwrap_or("");
                let content = r["content"].as_str().unwrap_or("");
                out.push_str(&format!("[{}] {}\n{}\n{}\n\n", i + 1, title, url, content));
            }
        }
        if out.is_empty() {
            out.push_str("No results.");
        }
        out
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description:
                "Search the web for a query and return ranked results with snippets and URLs."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Search query" } },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<String, ToolError> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| ToolError::BadArgs("missing 'query'".into()))?;
        let body = json!({ "api_key": self.api_key, "query": query, "max_results": 5 });
        let resp = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(Self::format_results(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_results() {
        let v = json!({"results":[{"title":"Rust","url":"https://rust-lang.org","content":"systems lang"}]});
        let out = WebSearch::format_results(&v);
        assert!(out.contains("Rust"));
        assert!(out.contains("https://rust-lang.org"));
    }

    #[test]
    fn empty_results_message() {
        assert_eq!(
            WebSearch::format_results(&json!({"results":[]})),
            "No results."
        );
    }

    #[test]
    fn spec_requires_query() {
        let t = WebSearch::tavily("k");
        assert_eq!(t.spec().name, "web_search");
        assert_eq!(t.spec().parameters["required"][0], "query");
    }
}
