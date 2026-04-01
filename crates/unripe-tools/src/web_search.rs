use async_trait::async_trait;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

pub struct WebSearchTool {
    client: reqwest::Client,
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::limited(3))
                .user_agent("unripe-agent/0.3")
                .build()
                .expect("Failed to create HTTP client"),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using DuckDuckGo and return a summary of results. \
         Returns titles, URLs, and snippets for the top results."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        // Use DuckDuckGo HTML lite (no JS required)
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let response = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => {
                return Ok(ToolResult::Failure("Search timed out".into()));
            }
            Err(e) => {
                return Ok(ToolResult::Failure(format!("Search failed: {e}")));
            }
        };

        if !response.status().is_success() {
            return Ok(ToolResult::Failure(format!(
                "Search returned HTTP {}",
                response.status()
            )));
        }

        let html = match response.text().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolResult::Failure(format!(
                    "Failed to read search results: {e}"
                )));
            }
        };

        // Parse results from DuckDuckGo HTML lite
        let results = parse_ddg_results(&html, max_results);

        if results.is_empty() {
            return Ok(ToolResult::Success(format!(
                "No results found for: {query}"
            )));
        }

        let mut output = format!("Search results for: {query}\n\n");
        for (i, result) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. {}\n   {}\n   {}\n\n",
                i + 1,
                result.title,
                result.url,
                result.snippet
            ));
        }

        Ok(ToolResult::Success(output))
    }
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Parse DuckDuckGo HTML lite results using simple string matching.
/// No HTML parser dependency needed — DDG lite has a predictable structure.
fn parse_ddg_results(html: &str, max: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // DDG HTML lite uses <a class="result__a" href="...">title</a>
    // and <a class="result__snippet" ...>snippet</a>
    let mut pos = 0;
    while results.len() < max {
        // Find next result link
        let link_marker = "class=\"result__a\"";
        let link_start = match html[pos..].find(link_marker) {
            Some(i) => pos + i,
            None => break,
        };

        // Extract href
        let href_start = match html[..link_start].rfind("href=\"") {
            Some(i) => i + 6,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let href_end = match html[href_start..].find('"') {
            Some(i) => href_start + i,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let raw_url = &html[href_start..href_end];

        // Extract URL from DDG redirect (//duckduckgo.com/l/?uddg=ENCODED_URL&...)
        let url = if let Some(uddg_pos) = raw_url.find("uddg=") {
            let encoded = &raw_url[uddg_pos + 5..];
            let end = encoded.find('&').unwrap_or(encoded.len());
            urlencoding::decode(&encoded[..end])
                .unwrap_or_default()
                .to_string()
        } else {
            raw_url.to_string()
        };

        // Extract title (text between > and </a>)
        let title_start = match html[link_start..].find('>') {
            Some(i) => link_start + i + 1,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title_end = match html[title_start..].find("</a>") {
            Some(i) => title_start + i,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title = strip_html_tags(&html[title_start..title_end]);

        // Find snippet
        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(snip_start) = html[title_end..].find(snippet_marker) {
            let snip_abs = title_end + snip_start;
            if let Some(tag_end) = html[snip_abs..].find('>') {
                let text_start = snip_abs + tag_end + 1;
                if let Some(text_end) = html[text_start..].find("</") {
                    strip_html_tags(&html[text_start..text_start + text_end])
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }

        pos = title_end;
    }

    results
}

/// Remove HTML tags from a string
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
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
        let tool = WebSearchTool::new();
        let def = tool.to_definition();
        assert_eq!(def.name, "web_search");
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    #[tokio::test]
    async fn test_missing_query() {
        let tool = WebSearchTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("hello <b>world</b>"), "hello world");
        assert_eq!(strip_html_tags("a &amp; b"), "a & b");
        assert_eq!(strip_html_tags("<a href=\"x\">link</a>"), "link");
    }

    #[test]
    fn test_parse_ddg_empty() {
        let results = parse_ddg_results("no results here", 5);
        assert!(results.is_empty());
    }
}
