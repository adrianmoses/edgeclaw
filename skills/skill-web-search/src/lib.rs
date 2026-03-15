use mcp_server_util::{
    initialize_result, tool_call_result, tools_list_result, JsonRpcRequest, JsonRpcResponse,
    ToolDef,
};
use serde::Deserialize;
use worker::*;

fn tool_definitions() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "web_search",
        description: "Search the web using Brave Search API",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "max_results": { "type": "integer", "description": "Maximum results (1-10, default 5)" }
            },
            "required": ["query"]
        }),
    }]
}

#[derive(Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Deserialize)]
struct BraveSearchResponse {
    #[serde(default)]
    web: Option<BraveWebResults>,
}

#[derive(Deserialize)]
struct BraveWebResults {
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    #[serde(default)]
    description: Option<String>,
}

async fn do_search(query: &str, max_results: usize, api_key: &str) -> serde_json::Value {
    let count = max_results.clamp(1, 10);
    let encoded_query = urlencoding::encode(query);
    let url =
        format!("https://api.search.brave.com/res/v1/web/search?q={encoded_query}&count={count}");

    let mut init = RequestInit::new();
    init.method = Method::Get;
    init.headers
        .set("Accept", "application/json")
        .unwrap_or_default();
    init.headers
        .set("Accept-Encoding", "gzip")
        .unwrap_or_default();
    init.headers
        .set("X-Subscription-Token", api_key)
        .unwrap_or_default();

    let request = match Request::new_with_init(&url, &init) {
        Ok(r) => r,
        Err(e) => return tool_call_result(&format!("Failed to create request: {e:?}"), true),
    };

    let mut response = match Fetch::Request(request).send().await {
        Ok(r) => r,
        Err(e) => return tool_call_result(&format!("Brave API request failed: {e:?}"), true),
    };

    let body = match response.text().await {
        Ok(t) => t,
        Err(e) => return tool_call_result(&format!("Failed to read response: {e:?}"), true),
    };

    let brave: BraveSearchResponse = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => return tool_call_result(&format!("Failed to parse Brave response: {e}"), true),
    };

    let results = match brave.web {
        Some(web) => web.results,
        None => return tool_call_result("No web results found", false),
    };

    let formatted: Vec<String> = results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            format!(
                "{}. **{}**\n   {}\n   {}",
                i + 1,
                r.title,
                r.url,
                r.description.as_deref().unwrap_or("")
            )
        })
        .collect();

    tool_call_result(&formatted.join("\n\n"), false)
}

#[event(fetch)]
async fn main(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
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
            JsonRpcResponse::success(body.id, initialize_result("skill-web-search", &tools))
        }
        "tools/list" => JsonRpcResponse::success(body.id, tools_list_result(&tools)),
        "tools/call" => {
            let params: ToolCallParams = body
                .params
                .as_ref()
                .and_then(|p| serde_json::from_value(p.clone()).ok())
                .ok_or_else(|| Error::RustError("Missing tools/call params".to_string()))?;

            if params.name != "web_search" {
                return Response::from_json(&JsonRpcResponse::error(
                    body.id,
                    -32602,
                    format!("Unknown tool: {}", params.name),
                ));
            }

            let query = params.arguments["query"].as_str().unwrap_or("").to_string();
            let max_results = params.arguments["max_results"].as_u64().unwrap_or(5) as usize;

            let api_key = match env.secret("BRAVE_SEARCH_API_KEY") {
                Ok(s) => s.to_string(),
                Err(_) => {
                    let result =
                        tool_call_result("BRAVE_SEARCH_API_KEY secret is not configured", true);
                    return Response::from_json(&JsonRpcResponse::success(body.id, result));
                }
            };

            let result = do_search(&query, max_results, &api_key).await;
            JsonRpcResponse::success(body.id, result)
        }
        _ => JsonRpcResponse::method_not_found(body.id, &body.method),
    };

    Response::from_json(&response)
}
