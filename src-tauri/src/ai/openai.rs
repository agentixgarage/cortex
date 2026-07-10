// OpenAI Chat Completions API direct reqwest implementation.
// Both API key and OAuth use Bearer auth.
// System prompt is a message with role=system at index 0.

use crate::ai::{AIServiceResponse, ServiceMessage};
use serde_json::{json, Value};

// ── ChatGPT/Codex Responses API ──
// [Outcome 1 chosen per 07-OAUTH-RESEARCH.md §Codex Chat Routing]
// Codex OAuth tokens route to chatgpt.com/backend-api/codex/responses (Responses API),
// NOT api.openai.com/v1/chat/completions (Chat Completions API).
// The two endpoints are incompatible: different wire format + different auth model.
//
// [ASSUMED per 07-OAUTH-RESEARCH.md §Codex Chat Routing Note]: The exact non-streaming
// Responses API shape. Using stream=false with the documented Responses API format
// ({"model":..., "input":[...], "stream": false, "max_output_tokens": ...}).

const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Build request headers and body for the ChatGPT/Codex Responses API.
///
/// Endpoint: POST https://chatgpt.com/backend-api/codex/responses
/// Auth: Bearer token (from Codex OAuth PKCE flow)
/// Wire format: OpenAI Responses API (non-streaming)
///
/// [ASSUMED] Request/response shape based on Responses API documentation.
/// The OpenAI Responses API uses "input" array (not "messages"), "max_output_tokens" (not "max_tokens"),
/// and response body has "output" array with text content.
pub fn build_codex_request(
    token: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[ServiceMessage],
) -> (String, Vec<(String, String)>, Value) {
    let url = format!("{}/responses", CHATGPT_CODEX_BASE_URL);

    let headers = vec![
        ("Authorization".to_string(), format!("Bearer {}", token)),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];

    // Responses API input format: system message as a developer role item,
    // then user/assistant turns. Non-streaming.
    let mut input: Vec<Value> = Vec::new();
    if !system.is_empty() {
        input.push(json!({"role": "developer", "content": system}));
    }
    for m in messages {
        input.push(json!({"role": m.role, "content": m.content}));
    }

    let body = json!({
        "model": model,
        "input": input,
        "max_output_tokens": max_tokens,
        "stream": false,
        // The Responses API defaults `store` to true (persists the response
        // server-side for 30 days, retrievable via response_id — a
        // Platform/API-key-tier feature). ChatGPT-subscription auth cannot
        // use that storage tier and rejects requests that omit this:
        // {"detail":"Store must be set to false"}.
        "store": false,
    });

    (url, headers, body)
}

/// Build request headers and body for the ChatGPT/Codex Responses API in streaming mode.
///
/// Identical to `build_codex_request` but explicitly sets `"stream": true`
/// (the non-streaming builder already sets `stream: false` — this toggles it).
/// Pure function — no I/O — for unit testability.
pub fn build_codex_stream_request(
    token: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[ServiceMessage],
) -> (String, Vec<(String, String)>, Value) {
    let (url, headers, mut body) = build_codex_request(token, model, max_tokens, system, messages);
    body["stream"] = json!(true);
    (url, headers, body)
}

/// Map ChatGPT backend HTTP status codes to user-friendly error messages.
fn map_codex_error(status: u16, body: &str) -> String {
    match status {
        401 => "ChatGPT token invalid or expired. Please sign in again via Settings.".to_string(),
        403 => "ChatGPT token does not have the required permissions.".to_string(),
        429 => "ChatGPT rate limit — try again shortly.".to_string(),
        500..=599 => "ChatGPT service unavailable. Try again later.".to_string(),
        _ => {
            if let Ok(json) = serde_json::from_str::<Value>(body) {
                if let Some(msg) = json["error"]["message"].as_str() {
                    return msg.chars().take(200).collect();
                }
            }
            format!("ChatGPT Codex error ({}): {}", status, &body[..body.len().min(200)])
        }
    }
}

/// Send a chat request to the ChatGPT/Codex Responses API.
///
/// Uses a Bearer token from the Codex PKCE OAuth flow (NOT an API key).
/// Endpoint: https://chatgpt.com/backend-api/codex/responses
/// Wire format: OpenAI Responses API (non-streaming, stream=false)
///
/// This function must NOT be called for "openai" (API-key) credentials —
/// those must use openai_chat() (api.openai.com/v1/chat/completions).
pub async fn codex_chat(
    token: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[ServiceMessage],
) -> Result<AIServiceResponse, String> {
    let (url, headers, body) = build_codex_request(token, model, max_tokens, system, messages);

    let client = reqwest::Client::new();
    let mut req = client.post(&url);
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let res = req
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                "Could not reach ChatGPT — check your connection.".to_string()
            } else {
                format!("Network error: {}", e)
            }
        })?;

    let status = res.status().as_u16();
    let text = res.text().await.map_err(|e| format!("Read error: {}", e))?;

    if status != 200 {
        return Err(map_codex_error(status, &text));
    }

    let json: Value = serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;

    // Responses API response shape: { "output": [{"type":"message","content":[{"type":"output_text","text":"..."}]}] }
    // Extract text from the first output message's first content item.
    let content = json["output"]
        .as_array()
        .and_then(|arr| arr.iter().find(|item| item["type"] == "message"))
        .and_then(|msg| msg["content"].as_array())
        .and_then(|parts| parts.iter().find(|p| p["type"] == "output_text"))
        .and_then(|part| part["text"].as_str())
        .unwrap_or("")
        .to_string();

    let input_tokens = json["usage"]["input_tokens"].as_u64();
    let output_tokens = json["usage"]["output_tokens"].as_u64();

    Ok(AIServiceResponse {
        content,
        model: json["model"].as_str().unwrap_or(model).to_string(),
        input_tokens,
        output_tokens,
    })
}

/// Build request headers and body for the OpenAI Chat Completions API.
///
/// Returns (url, headers_vec, body_json) where headers_vec is (name, value) pairs.
/// Pure function — no I/O — for unit testability without network access.
pub fn build_openai_request(
    token: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[ServiceMessage],
) -> (String, Vec<(String, String)>, Value) {
    let url = "https://api.openai.com/v1/chat/completions".to_string();

    let headers = vec![
        ("Authorization".to_string(), format!("Bearer {}", token)),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];

    // OpenAI Chat format: system message at index 0, then user/assistant
    let mut messages_json: Vec<Value> = Vec::with_capacity(messages.len() + 1);
    messages_json.push(json!({"role": "system", "content": system}));
    for m in messages {
        messages_json.push(json!({"role": m.role, "content": m.content}));
    }

    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages_json,
    });

    (url, headers, body)
}

/// Build request headers and body for the OpenAI Chat Completions API in streaming mode.
///
/// Reuses `build_openai_request` but sets `"stream": true` and
/// `"stream_options": {"include_usage": true}` — the latter is required to
/// receive token counts in the final SSE chunk (OpenAI omits usage from
/// streamed responses unless explicitly requested).
/// Pure function — no I/O — for unit testability.
pub fn build_openai_stream_request(
    token: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[ServiceMessage],
) -> (String, Vec<(String, String)>, Value) {
    let (url, headers, mut body) = build_openai_request(token, model, max_tokens, system, messages);
    body["stream"] = json!(true);
    body["stream_options"] = json!({ "include_usage": true });
    (url, headers, body)
}

/// Map OpenAI HTTP status codes to user-friendly error messages.
fn map_openai_error(status: u16, body: &str) -> String {
    match status {
        401 => "Invalid OpenAI API key or bearer token.".to_string(),
        403 => "Token does not have the required permissions.".to_string(),
        429 => "OpenAI rate limit — try again shortly.".to_string(),
        500..=599 => "OpenAI service unavailable. Try again later.".to_string(),
        _ => {
            if let Ok(json) = serde_json::from_str::<Value>(body) {
                if let Some(msg) = json["error"]["message"].as_str() {
                    return msg.chars().take(200).collect();
                }
            }
            format!("OpenAI API error ({}): {}", status, &body[..body.len().min(200)])
        }
    }
}

/// Send a chat request to the OpenAI Chat Completions API.
///
/// Works for both API key auth and OAuth bearer tokens.
/// For `openai-codex` provider (ChatGPT subscription): caller passes the OAuth token as `token`.
pub async fn openai_chat(
    token: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[ServiceMessage],
) -> Result<AIServiceResponse, String> {
    let (url, headers, body) = build_openai_request(token, model, max_tokens, system, messages);

    let client = reqwest::Client::new();
    let mut req = client.post(&url);
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let res = req
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                "Could not reach OpenAI — check your connection.".to_string()
            } else {
                format!("Network error: {}", e)
            }
        })?;

    let status = res.status().as_u16();
    let text = res.text().await.map_err(|e| format!("Read error: {}", e))?;

    if status != 200 {
        return Err(map_openai_error(status, &text));
    }

    let json: Value = serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;

    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let total_tokens = json["usage"]["total_tokens"].as_u64().unwrap_or(0);
    let input_tokens = json["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = json["usage"]["completion_tokens"].as_u64().unwrap_or(0);

    // Note: total_tokens = input_tokens + output_tokens from OpenAI
    let _ = total_tokens;

    Ok(AIServiceResponse {
        content,
        model: json["model"].as_str().unwrap_or(model).to_string(),
        input_tokens: Some(input_tokens),
        output_tokens: Some(output_tokens),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ServiceMessage;

    fn sample_messages() -> Vec<ServiceMessage> {
        vec![ServiceMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        }]
    }

    // Test 1: Bearer auth header and endpoint
    #[test]
    fn test_build_request_bearer_auth() {
        let msgs = sample_messages();
        let (url, headers, body) = build_openai_request("sk-test", "gpt-4o", 100, "sys", &msgs);

        assert_eq!(url, "https://api.openai.com/v1/chat/completions");

        let auth = headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.as_str());
        assert_eq!(auth, Some("Bearer sk-test"), "Got: {:?}", headers);

        // Ensure body has no top-level "system" field (OpenAI shape)
        assert!(body.get("system").is_none(), "OpenAI body must not have top-level system field");
    }

    // Test 2: system prompt is first message with role=system
    #[test]
    fn test_build_request_system_as_first_message() {
        let msgs = sample_messages();
        let (_, _, body) = build_openai_request("sk-test", "gpt-4o", 100, "system content", &msgs);

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "system content");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "hello");
    }

    // Test 3: model field in body
    #[test]
    fn test_build_request_model_field() {
        let msgs = sample_messages();
        let (_, _, body) = build_openai_request("sk-test", "gpt-4o", 256, "sys", &msgs);

        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["max_tokens"], 256);
    }

    // Test 4: Error mapping
    #[test]
    fn test_map_openai_error_401() {
        let msg = map_openai_error(401, "");
        assert!(msg.contains("Invalid OpenAI"), "Got: {}", msg);
    }

    #[test]
    fn test_map_openai_error_429() {
        let msg = map_openai_error(429, "");
        assert!(msg.contains("rate limit"), "Got: {}", msg);
    }

    #[test]
    fn test_map_openai_error_uses_body_message() {
        let body = r#"{"error":{"message":"model not found","type":"invalid_request_error"}}"#;
        let msg = map_openai_error(400, body);
        assert!(msg.contains("model not found"), "Got: {}", msg);
    }

    // --- Plan 07-09 Task 3a: codex_chat endpoint and request shape tests ---

    #[test]
    fn test_build_codex_request_uses_chatgpt_endpoint() {
        let msgs = sample_messages();
        let (url, headers, body) = build_codex_request("bearer_tok", "gpt-5", 100, "sys", &msgs);

        // Must NOT use api.openai.com (that's for API-key auth)
        assert!(
            url.contains("chatgpt.com/backend-api/codex/responses"),
            "Codex endpoint must be chatgpt.com/backend-api/codex/responses, got: {}",
            url
        );

        let auth = headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.as_str());
        assert_eq!(auth, Some("Bearer bearer_tok"), "Got: {:?}", headers);

        // Responses API shape: "input" array, not "messages"
        assert!(body.get("input").is_some(), "Codex body must have 'input' field (Responses API)");
        assert!(body.get("messages").is_none(), "Codex body must NOT have 'messages' (Chat Completions API field)");
        assert!(body.get("stream").is_some(), "Codex body must have 'stream' field");
        assert_eq!(body["stream"], false, "stream must be false for non-streaming");
        assert_eq!(body["max_output_tokens"], 100, "max_output_tokens must be set");
    }

    #[test]
    fn test_build_codex_request_sets_store_false() {
        // Real bug: omitting `store` defaults to true on the Responses API
        // (persists server-side, a Platform/API-key-tier feature). ChatGPT
        // subscription auth rejects that with {"detail":"Store must be set
        // to false"}.
        let msgs = sample_messages();
        let (_, _, body) = build_codex_request("tok", "gpt-5.6-terra", 100, "sys", &msgs);
        assert_eq!(body["store"], false, "store must be explicitly false for ChatGPT-auth requests");
    }

    #[test]
    fn test_build_codex_stream_request_also_sets_store_false() {
        let msgs = sample_messages();
        let (_, _, body) = build_codex_stream_request("tok", "gpt-5.6-terra", 100, "sys", &msgs);
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn test_build_codex_request_system_as_developer_role() {
        let msgs = sample_messages();
        let (_, _, body) = build_codex_request("tok", "gpt-5", 100, "system content", &msgs);

        let input = body["input"].as_array().unwrap();
        // Developer role for system prompt (Responses API convention)
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["content"], "system content");
        // User message follows
        assert_eq!(input[1]["role"], "user");
        assert_eq!(input[1]["content"], "hello");
    }

    #[test]
    fn test_build_codex_request_skips_empty_system() {
        let msgs = sample_messages();
        let (_, _, body) = build_codex_request("tok", "gpt-5", 100, "", &msgs);

        let input = body["input"].as_array().unwrap();
        // Empty system prompt → no developer role item, only user message
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
    }

    // --- Plan 11.7-04 Task 3: streaming request builder tests ---

    #[test]
    fn test_build_openai_stream_request_adds_stream_flag() {
        let msgs = sample_messages();
        let (url, _, body) = build_openai_stream_request("sk-test", "gpt-4o", 100, "sys", &msgs);

        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn test_build_codex_stream_request_sets_stream_true() {
        let msgs = sample_messages();
        let (url, _, body) = build_codex_stream_request("tok", "gpt-5", 100, "sys", &msgs);

        assert!(url.contains("chatgpt.com/backend-api/codex/responses"));
        assert_eq!(body["stream"], true);
    }
}
