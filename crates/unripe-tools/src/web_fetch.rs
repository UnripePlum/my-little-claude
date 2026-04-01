use std::net::IpAddr;

use async_trait::async_trait;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::limited(5))
                .user_agent("unripe-agent/0.3")
                .build()
                .expect("Failed to create HTTP client"),
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch the content of a URL via HTTP GET. Returns the response body as text. \
         Useful for reading documentation, APIs, or web pages."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum response size in bytes (default: 100000)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        let max_bytes = input
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(100_000) as usize;

        // URL scheme validation
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(ToolResult::Failure(
                "URL must start with http:// or https://".into(),
            ));
        }

        // SSRF protection: block private/internal addresses + DNS rebinding
        if let Err(reason) = check_ssrf(url).await {
            return Ok(ToolResult::Failure(reason));
        }

        let response = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => {
                return Ok(ToolResult::Failure(format!("Request timed out: {url}")));
            }
            Err(e) if e.is_connect() => {
                return Ok(ToolResult::Failure(format!("Connection failed: {e}")));
            }
            Err(e) => {
                return Ok(ToolResult::Failure(format!("Request failed: {e}")));
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Ok(ToolResult::Failure(format!("HTTP {status} for {url}")));
        }

        // Read body incrementally to avoid OOM on large responses
        let content_len = response.content_length().unwrap_or(0) as usize;
        let cap = max_bytes.min(content_len.max(4096));
        let mut buf = Vec::with_capacity(cap);
        let mut stream = response.bytes_stream();

        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolResult::Failure(format!(
                        "Failed to read response body: {e}"
                    )));
                }
            };
            buf.extend_from_slice(&chunk);
            if buf.len() >= max_bytes {
                buf.truncate(max_bytes);
                let truncated = String::from_utf8_lossy(&buf);
                return Ok(ToolResult::Success(format!(
                    "{truncated}\n\n[Truncated at {max_bytes} bytes]"
                )));
            }
        }

        let body = String::from_utf8_lossy(&buf).to_string();
        Ok(ToolResult::Success(body))
    }
}

/// Check if a URL targets a private/internal address (SSRF protection).
/// Performs both hostname blocklist and DNS resolution check to prevent
/// DNS rebinding attacks (e.g., evil.com resolving to 127.0.0.1).
async fn check_ssrf(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;

    let host = parsed.host_str().unwrap_or("");
    let port = parsed.port_or_known_default().unwrap_or(80);

    // Block known internal hostnames
    let blocked_hosts = [
        "localhost",
        "metadata.google.internal",
        "metadata",
        "169.254.169.254",
    ];
    let host_lower = host.to_lowercase();
    for blocked in &blocked_hosts {
        if host_lower == *blocked || host_lower.ends_with(&format!(".{blocked}")) {
            return Err(format!("Requests to internal host '{host}' are blocked"));
        }
    }

    // Check if host is a literal IP
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(format!("Requests to private IP {ip} are blocked"));
        }
    }

    // DNS resolution check: resolve hostname and verify all IPs are public.
    // This prevents DNS rebinding attacks where a public hostname resolves to 127.0.0.1.
    let addr = format!("{host}:{port}");
    match tokio::net::lookup_host(&addr).await {
        Ok(addrs) => {
            for addr in addrs {
                if is_private_ip(addr.ip()) {
                    return Err(format!(
                        "DNS for '{host}' resolves to private IP {} — blocked",
                        addr.ip()
                    ));
                }
            }
        }
        Err(_) => {
            // DNS resolution failed — let reqwest handle the error naturally
        }
    }

    Ok(())
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()             // 127.0.0.0/8
                || v4.is_private()       // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()    // 169.254.0.0/16
                || v4.is_unspecified()   // 0.0.0.0
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGNAT)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()             // ::1
                || v6.is_unspecified()   // ::
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_ctx() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            session_id: "test".into(),
            env: HashMap::new(),
        }
    }

    #[test]
    fn test_tool_definition() {
        let tool = WebFetchTool::new();
        let def = tool.to_definition();
        assert_eq!(def.name, "web_fetch");
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "url"));
    }

    #[tokio::test]
    async fn test_invalid_url_scheme() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(serde_json::json!({"url": "ftp://example.com"}), &test_ctx())
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("http")));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = WebFetchTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ssrf_localhost_blocked() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(
                serde_json::json!({"url": "http://localhost/admin"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("blocked")));
    }

    #[tokio::test]
    async fn test_ssrf_private_ip_blocked() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(
                serde_json::json!({"url": "http://192.168.1.1/"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("blocked")));
    }

    #[tokio::test]
    async fn test_ssrf_metadata_blocked() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(
                serde_json::json!({"url": "http://169.254.169.254/latest/meta-data/"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("blocked")));
    }

    #[tokio::test]
    async fn test_ssrf_loopback_blocked() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(
                serde_json::json!({"url": "http://127.0.0.1:8080/"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("blocked")));
    }

    #[tokio::test]
    async fn test_check_ssrf_allows_public() {
        assert!(check_ssrf("https://example.com").await.is_ok());
        assert!(check_ssrf("https://api.github.com/repos").await.is_ok());
    }

    #[tokio::test]
    async fn test_check_ssrf_blocks_private() {
        assert!(check_ssrf("http://localhost/").await.is_err());
        assert!(check_ssrf("http://127.0.0.1/").await.is_err());
        assert!(check_ssrf("http://10.0.0.1/").await.is_err());
        assert!(check_ssrf("http://172.16.0.1/").await.is_err());
        assert!(check_ssrf("http://192.168.1.1/").await.is_err());
        assert!(check_ssrf("http://169.254.169.254/").await.is_err());
        assert!(check_ssrf("http://[::1]/").await.is_err());
    }

    #[test]
    fn test_is_private_ip() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.1".parse().unwrap()));
        assert!(is_private_ip("169.254.169.254".parse().unwrap()));
        assert!(is_private_ip("::1".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
    }
}
