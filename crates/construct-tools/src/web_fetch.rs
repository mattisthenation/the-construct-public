use async_trait::async_trait;
use construct_core::tool::{Tool, ToolError, ToolSpec};
use serde_json::{json, Value};

/// Fetch a URL and return readable text (very small HTML→text reduction).
pub struct WebFetch {
    http: reqwest::Client,
    max_chars: usize,
    max_bytes: usize,
}

/// True if an IP is one we must never fetch from (loopback, private LAN,
/// link-local incl. cloud metadata 169.254.169.254, unspecified). Blocks the
/// obvious SSRF targets a note/web-page URL could point at.
fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

/// Reject non-http(s) schemes and URLs whose host resolves to a blocked address.
/// Async (uses `tokio::net::lookup_host`) so the DNS lookup never blocks the runtime.
async fn check_url_safe(url: &str) -> Result<(), ToolError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ToolError::BadArgs(format!(
                "unsupported URL scheme '{other}'"
            )))
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::BadArgs("URL has no host".into()))?;
    // If the host is an IP literal, check it directly; otherwise resolve (async) and
    // check every address it maps to (reject if ANY is blocked).
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(ToolError::Failed(format!(
                "refusing to fetch internal address {ip}"
            )));
        }
    } else {
        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut any = false;
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        for addr in addrs {
            any = true;
            if is_blocked_ip(addr.ip()) {
                return Err(ToolError::Failed(format!(
                    "refusing to fetch {host} (resolves to internal address {})",
                    addr.ip()
                )));
            }
        }
        if !any {
            return Err(ToolError::Failed(format!("could not resolve host {host}")));
        }
    }
    Ok(())
}

impl WebFetch {
    pub fn new() -> Self {
        // Finite timeouts so a slow/unresponsive host can't stall the pipeline. Redirects
        // are disabled here and followed manually in `call`, so every hop (including
        // hostname redirects) is screened by `check_url_safe` for SSRF.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        WebFetch {
            http,
            max_chars: 8000,
            max_bytes: 2_000_000,
        }
    }

    /// Pure: strip tags/scripts/styles and collapse whitespace. UTF-8 safe.
    ///
    /// Iterates over `char`s (not bytes) so multi-byte characters such as
    /// em-dashes never cause a panic or get mangled. (The previous byte-index
    /// version sliced `str` at arbitrary byte offsets and panicked mid-char.)
    pub fn html_to_text(html: &str) -> String {
        let chars: Vec<char> = html.chars().collect();
        let mut out = String::with_capacity(html.len());
        let mut in_tag = false;
        let mut skip_block: Option<&'static str> = None;
        let mut i = 0;
        while i < chars.len() {
            if let Some(tag) = skip_block {
                // Look for the matching close tag, e.g. </script>.
                if matches_ci(&chars, i, "</")
                    && matches_ci(&chars, i + 2, tag)
                    && chars.get(i + 2 + tag.len()) == Some(&'>')
                {
                    skip_block = None;
                    i += 2 + tag.len() + 1;
                    in_tag = false;
                } else {
                    i += 1;
                }
                continue;
            }
            if matches_ci(&chars, i, "<script") {
                skip_block = Some("script");
                i += "<script".len();
                in_tag = true;
                continue;
            }
            if matches_ci(&chars, i, "<style") {
                skip_block = Some("style");
                i += "<style".len();
                in_tag = true;
                continue;
            }
            match chars[i] {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => out.push(c),
                _ => {}
            }
            i += 1;
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Char-boundary-safe truncation to at most `max_chars` characters.
    /// (`String::truncate` panics on a non-char-boundary byte index.)
    pub fn truncate_chars(s: &str, max_chars: usize) -> String {
        s.chars().take(max_chars).collect()
    }
}

/// Case-insensitive ASCII match of `pat` at char position `i` in `chars`.
fn matches_ci(chars: &[char], i: usize, pat: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    if i + pat.len() > chars.len() {
        return false;
    }
    pat.iter()
        .enumerate()
        .all(|(k, p)| chars[i + k].eq_ignore_ascii_case(p))
}

impl Default for WebFetch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "Fetch a URL and return its readable text content.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "Absolute URL to fetch" } },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<String, ToolError> {
        use futures::StreamExt;
        let start = args["url"]
            .as_str()
            .ok_or_else(|| ToolError::BadArgs("missing 'url'".into()))?;
        // Follow redirects manually so EVERY hop is SSRF-screened (reqwest auto-redirect
        // is disabled). Each hop is checked with check_url_safe before the request.
        let mut url = start.to_string();
        let mut hops = 0u32;
        let resp = loop {
            check_url_safe(&url).await?;
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| ToolError::Failed(e.to_string()))?;
            if resp.status().is_redirection() && hops < 3 {
                if let Some(loc) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                {
                    // Resolve relative Location against the current URL.
                    let next = reqwest::Url::parse(&url)
                        .ok()
                        .and_then(|base| base.join(loc).ok())
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| loc.to_string());
                    url = next;
                    hops += 1;
                    continue;
                }
            }
            break resp;
        };
        // Read the body with a hard byte cap so a huge/streaming response can't OOM us.
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ToolError::Failed(e.to_string()))?;
            buf.extend_from_slice(&chunk);
            if buf.len() >= self.max_bytes {
                buf.truncate(self.max_bytes);
                break;
            }
        }
        let html = String::from_utf8_lossy(&buf);
        let text = Self::truncate_chars(&Self::html_to_text(&html), self.max_chars);
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_internal_ip_literals() {
        use std::net::IpAddr;
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "::1",
        ] {
            assert!(
                is_blocked_ip(ip.parse::<IpAddr>().unwrap()),
                "{ip} should be blocked"
            );
        }
        for ip in ["8.8.8.8", "1.1.1.1"] {
            assert!(
                !is_blocked_ip(ip.parse::<IpAddr>().unwrap()),
                "{ip} should be allowed"
            );
        }
    }

    #[tokio::test]
    async fn check_url_safe_rejects_internal_and_bad_schemes() {
        // Internal IP-literal hosts are refused.
        assert!(check_url_safe("http://127.0.0.1:11434/api").await.is_err());
        assert!(check_url_safe("http://192.168.1.50/admin").await.is_err());
        assert!(check_url_safe("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        // Non-http(s) schemes are refused.
        assert!(check_url_safe("file:///etc/passwd").await.is_err());
        assert!(check_url_safe("ftp://example.com/x").await.is_err());
        // A public IP literal passes the guard.
        assert!(check_url_safe("http://1.1.1.1/").await.is_ok());
    }

    #[test]
    fn strips_tags_and_scripts() {
        let html = "<html><head><style>x{}</style></head><body><p>Hello</p><script>var a=1;</script> world</body></html>";
        let text = WebFetch::html_to_text(html);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn spec_requires_url() {
        assert_eq!(WebFetch::new().spec().parameters["required"][0], "url");
    }

    #[test]
    fn handles_multibyte_chars_without_panic() {
        // Reproduces the live panic: an em-dash (3 bytes) inside a skipped
        // <style> block AND in body text. Byte-index slicing used to land
        // mid-character and panic; byte-as-char used to mangle é/—.
        let html = "<style>/* — */</style><body>Café — résumé</body>";
        let text = WebFetch::html_to_text(html);
        assert_eq!(text, "Café — résumé");
    }

    #[test]
    fn truncates_on_char_boundary() {
        let s = "—".repeat(10); // 30 bytes, 10 chars
        let out = WebFetch::truncate_chars(&s, 5);
        assert_eq!(out.chars().count(), 5);
        assert_eq!(out, "—————");
    }
}
