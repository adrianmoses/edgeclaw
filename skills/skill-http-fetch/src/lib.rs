use mcp_server_util::{
    initialize_result, tool_call_result, tools_list_result, JsonRpcRequest, JsonRpcResponse,
    ToolDef,
};
use serde::Deserialize;
use worker::*;

const MAX_BODY_SIZE: usize = 100 * 1024; // 100KB

fn tool_definitions() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "http_fetch",
        description: "Fetch a URL and return its text content (HTML stripped/truncated to 100KB)",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch" },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional domain allowlist. If empty, all domains allowed."
                }
            },
            "required": ["url"]
        }),
    }]
}

#[derive(Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut last_was_space = false;

    let lower = html.to_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag
            && i + 7 <= lower_chars.len()
            && lower_chars[i..i + 7].iter().collect::<String>() == "<script"
        {
            in_script = true;
            in_tag = true;
            i += 1;
            continue;
        }
        if in_script
            && i + 9 <= lower_chars.len()
            && lower_chars[i..i + 9].iter().collect::<String>() == "</script>"
        {
            in_script = false;
            i += 9;
            continue;
        }
        if !in_tag
            && i + 6 <= lower_chars.len()
            && lower_chars[i..i + 6].iter().collect::<String>() == "<style"
        {
            in_style = true;
            in_tag = true;
            i += 1;
            continue;
        }
        if in_style
            && i + 8 <= lower_chars.len()
            && lower_chars[i..i + 8].iter().collect::<String>() == "</style>"
        {
            in_style = false;
            i += 8;
            continue;
        }

        if in_script || in_style {
            i += 1;
            continue;
        }

        if chars[i] == '<' {
            in_tag = true;
        } else if chars[i] == '>' {
            in_tag = false;
        } else if !in_tag {
            let ch = chars[i];
            if ch.is_whitespace() {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            } else {
                result.push(ch);
                last_was_space = false;
            }
        }
        i += 1;
    }

    result.trim().to_string()
}

async fn do_fetch(url: &str, allowed_domains: &[String]) -> serde_json::Value {
    // Validate URL
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(e) => return tool_call_result(&format!("Invalid URL: {e}"), true),
    };

    // Check domain allowlist
    if !allowed_domains.is_empty() {
        let host = parsed.host_str().unwrap_or("");
        if !allowed_domains
            .iter()
            .any(|d| host == d || host.ends_with(&format!(".{d}")))
        {
            return tool_call_result(&format!("Domain '{host}' not in allowlist"), true);
        }
    }

    let request = match Request::new(url, Method::Get) {
        Ok(r) => r,
        Err(e) => return tool_call_result(&format!("Failed to create request: {e:?}"), true),
    };

    let mut response = match Fetch::Request(request).send().await {
        Ok(r) => r,
        Err(e) => return tool_call_result(&format!("Fetch failed: {e:?}"), true),
    };

    let status = response.status_code();
    if status >= 400 {
        return tool_call_result(&format!("HTTP {status}"), true);
    }

    let text = match response.text().await {
        Ok(t) => t,
        Err(e) => return tool_call_result(&format!("Failed to read body: {e:?}"), true),
    };

    // Strip HTML and truncate
    let content_type = response
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .unwrap_or_default();

    let text = if content_type.contains("html") {
        strip_html(&text)
    } else {
        text
    };

    let truncated = if text.len() > MAX_BODY_SIZE {
        // Find a safe UTF-8 boundary to avoid panicking on multi-byte chars
        let mut end = MAX_BODY_SIZE;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...\n[truncated at 100KB]", &text[..end])
    } else {
        text
    };

    tool_call_result(&truncated, false)
}

#[event(fetch)]
async fn main(mut req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Post || req.path().as_str() != "/mcp" {
        return Response::error("POST /mcp only", 404);
    }

    let body: JsonRpcRequest = req
        .json()
        .await
        .map_err(|e| Error::RustError(format!("Invalid JSON-RPC: {e:?}")))?;

    let tools = tool_definitions();
    let response = match body.method.as_str() {
        "initialize" => {
            JsonRpcResponse::success(body.id, initialize_result("skill-http-fetch", &tools))
        }
        "tools/list" => JsonRpcResponse::success(body.id, tools_list_result(&tools)),
        "tools/call" => {
            let params: ToolCallParams = body
                .params
                .as_ref()
                .and_then(|p| serde_json::from_value(p.clone()).ok())
                .ok_or_else(|| Error::RustError("Missing tools/call params".to_string()))?;

            if params.name != "http_fetch" {
                return Response::from_json(&JsonRpcResponse::error(
                    body.id,
                    -32602,
                    format!("Unknown tool: {}", params.name),
                ));
            }

            let url = params.arguments["url"].as_str().unwrap_or("").to_string();
            let allowed_domains: Vec<String> = params.arguments["allowed_domains"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let result = do_fetch(&url, &allowed_domains).await;
            JsonRpcResponse::success(body.id, result)
        }
        _ => JsonRpcResponse::method_not_found(body.id, &body.method),
    };

    Response::from_json(&response)
}
