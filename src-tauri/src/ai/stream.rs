//! Streaming variant of ai_request().
//!
//! Reuses AIServiceRequest and the credential dispatch logic from service.rs.
//! Instead of returning a single AIServiceResponse, this module returns an
//! async Stream of StreamChunk items so the caller can forward each token to
//! the frontend via Tauri events (chat-stream-token, chat-stream-complete).
//!
//! Per D-08 (11.7-CONTEXT.md): all four providers stream via SSE-style HTTP.
//! Ollama falls back to a single final Done chunk if the local instance
//! disables streaming.
//!
//! **Retry / 401 handling:** Streaming does NOT retry on 401, unlike
//! `ai_request_with_retry`. SSE cannot be transparently replayed after the
//! first byte flushes to the caller. If a stream 401s on the first HTTP
//! status, this module emits `StreamChunk::Error` and terminates. Preflight
//! refresh (which runs BEFORE the POST, same as `ai_request`) still fires —
//! it's the http-status 401 mid-stream case (T-11.7-11) that is intentionally
//! left unhandled in v1.

use crate::ai::service::AIServiceRequest;
use serde::{Deserialize, Serialize};

/// A single item emitted by the ai_request_stream() future.
///
/// The stream MUST terminate with exactly one Done (on success) or one Error
/// (on failure) — never both, never neither.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamChunk {
    /// A text delta. `token` may span multiple characters (providers batch).
    /// `cumulative_index` is the count of tokens emitted so far in this stream.
    Token { token: String, cumulative_index: u32 },
    /// Terminal success. Includes final usage where the provider reports it.
    Done {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        model: String,
    },
    /// Terminal failure. Stream ends after this chunk.
    Error { message: String },
}

/// Central streaming AI request dispatch. Mirrors `ai_request`'s credential
/// resolution + provider routing (see service.rs), but returns a `Stream` of
/// `StreamChunk` instead of a single `AIServiceResponse`.
///
/// **No retry wrapper:** unlike `ai_request_with_retry`, streaming does not
/// retry. A failed stream emits a single `StreamChunk::Error` and terminates.
/// Callers (ChatEngine) surface the error via the `chat-stream-error` Tauri
/// event. See module doc comment for the full 401/retry rationale.
pub async fn ai_request_stream(
    auth: &crate::auth::AuthState,
    request: AIServiceRequest,
) -> Result<
    std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>>,
    String,
> {
    // 1. Preflight refresh (reuse crate::ai::service::preflight_refresh_if_needed).
    let _ = crate::ai::service::preflight_refresh_if_needed(auth).await;

    // 2. Resolve active credential — error out if none, mirroring ai_request.
    let cred = auth
        .get_active_credential()?
        .ok_or("No AI provider configured. Go to Settings to connect one.")?;

    // 3. Codex intercept before normalize (mirror ai_request's raw-provider check).
    if cred.provider == "openai-codex" {
        let token = cred.oauth_token.as_deref().ok_or_else(|| {
            "No OAuth token stored for openai-codex. Go to Settings → Sign in with ChatGPT."
                .to_string()
        })?;
        let model = request
            .model_override
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| cred.model.as_deref().unwrap_or("gpt-5.6-terra"))
            .to_string();
        let max_tokens = request.max_tokens.unwrap_or(4096);
        return Ok(codex_chat_stream(
            token.to_string(),
            model,
            max_tokens,
            request.system_prompt.clone(),
            request.messages.clone(),
        ));
    }

    // 4. Normalize + dispatch (anthropic / openai / gemini / ollama).
    let base_provider = crate::ai::service::normalize_provider_name(&cred.provider);

    let credential: Option<String> = match cred.method {
        crate::auth::AuthMethod::ApiKey => cred.api_key.clone(),
        crate::auth::AuthMethod::OAuth => cred.oauth_token.clone(),
        crate::auth::AuthMethod::None => None,
    };

    if credential.is_none() && base_provider != "ollama" {
        return Err(format!(
            "No credentials stored for {}. Go to Settings and connect using \
             a subscription token (recommended) or API key.",
            cred.provider
        ));
    }

    // Model resolution exactly mirrors ai_request — no chat-specific default.
    let model = request
        .model_override
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| cred.model.as_deref().unwrap_or("auto"))
        .to_string();
    let max_tokens = request.max_tokens.unwrap_or(4096);
    let token = credential.unwrap_or_default();

    match base_provider.as_str() {
        "anthropic" => {
            let is_setup_token = cred.method == crate::auth::AuthMethod::OAuth;
            Ok(anthropic_chat_stream(
                token,
                is_setup_token,
                model,
                max_tokens,
                request.system_prompt.clone(),
                request.messages.clone(),
            ))
        }

        "openai" => Ok(openai_chat_stream(
            token,
            model,
            max_tokens,
            request.system_prompt.clone(),
            request.messages.clone(),
        )),

        "gemini" => {
            let is_oauth = cred.method == crate::auth::AuthMethod::OAuth;
            Ok(gemini_chat_stream(
                token,
                is_oauth,
                model,
                max_tokens,
                request.system_prompt.clone(),
                request.messages.clone(),
            ))
        }

        "ollama" => {
            let base_url = cred
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            Ok(ollama_chat_stream(
                base_url,
                model,
                request.system_prompt.clone(),
                request.messages.clone(),
            ))
        }

        other => Err(format!("Unknown AI provider: {}", other)),
    }
}

// ============================================================================
// Anthropic SSE streaming
// ============================================================================

/// Split a raw SSE byte buffer into complete frames + a remaining partial frame.
///
/// SSE frames are separated by a blank line (`\n\n`). This helper is shared by
/// all SSE-based providers (Anthropic, OpenAI, Codex, Gemini). It intentionally
/// discards unparseable frames rather than retrying (T-11.7-08 — DoS mitigation:
/// a malformed frame from a compromised endpoint must not infinite-loop or grow
/// the buffer unbounded).
///
/// Returns `(complete_frames, remaining_partial_buffer)`.
fn split_sse_frames(buffer: &str) -> (Vec<String>, String) {
    let mut frames: Vec<String> = buffer.split("\n\n").map(|s| s.to_string()).collect();
    let remainder = frames.pop().unwrap_or_default();
    (frames, remainder)
}

/// Parse a single SSE frame's `field: value` lines into (event_name, data_payload).
/// Multiple `data:` lines are joined with `\n` per the SSE spec.
fn parse_sse_frame(frame: &str) -> (Option<String>, String) {
    let mut event: Option<String> = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim());
        }
    }
    (event, data_lines.join("\n"))
}

/// Parse a complete (or partial — call repeatedly as data arrives) Anthropic SSE
/// payload into StreamChunk items. Pure function — no I/O — for unit testing.
///
/// Recognizes:
/// - `content_block_delta` events with `delta.type == "text_delta"` → Token
/// - `message_delta` events with `usage` → remembered for the terminal Done
/// - `message_stop` → Done with remembered usage + model
/// - `error` events → Error
pub fn parse_anthropic_sse_frames(input: &str) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let (frames, _remainder) = split_sse_frames(input);

    let mut cumulative_index: u32 = 0;
    let mut input_tokens: Option<u64> = None;
    let mut output_tokens: Option<u64> = None;
    let mut model = String::new();

    for frame in frames {
        if frame.trim().is_empty() {
            continue;
        }
        let (event, data) = parse_sse_frame(&frame);
        if data.is_empty() {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue, // unparseable frame — discard (T-11.7-08)
        };

        // Prefer the explicit `event:` field, fall back to the JSON `type` field.
        let event_type = event
            .clone()
            .unwrap_or_else(|| json["type"].as_str().unwrap_or("").to_string());

        match event_type.as_str() {
            "message_start" => {
                if let Some(m) = json["message"]["model"].as_str() {
                    model = m.to_string();
                }
                if let Some(v) = json["message"]["usage"]["input_tokens"].as_u64() {
                    input_tokens = Some(v);
                }
            }
            "content_block_delta" => {
                if json["delta"]["type"] == "text_delta" {
                    if let Some(text) = json["delta"]["text"].as_str() {
                        cumulative_index += 1;
                        chunks.push(StreamChunk::Token {
                            token: text.to_string(),
                            cumulative_index,
                        });
                    }
                }
            }
            "message_delta" => {
                if let Some(v) = json["usage"]["input_tokens"].as_u64() {
                    input_tokens = Some(v);
                }
                if let Some(v) = json["usage"]["output_tokens"].as_u64() {
                    output_tokens = Some(v);
                }
            }
            "message_stop" => {
                chunks.push(StreamChunk::Done {
                    input_tokens,
                    output_tokens,
                    model: model.clone(),
                });
            }
            "error" => {
                let message = json["error"]["message"]
                    .as_str()
                    .unwrap_or("Anthropic stream error")
                    .to_string();
                chunks.push(StreamChunk::Error { message });
            }
            _ => {}
        }
    }

    chunks
}

/// Stream a chat request to the Anthropic Messages API.
///
/// Mirrors `anthropic_chat` credential/header/body construction but sets
/// `"stream": true` and parses the SSE response incrementally instead of
/// awaiting a single JSON body.
///
/// **No 401 retry** — see module doc comment. A 401 on the initial HTTP
/// response is surfaced as a single `StreamChunk::Error`.
pub fn anthropic_chat_stream(
    token: String,
    is_setup_token: bool,
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<crate::ai::ServiceMessage>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>> {
    let stream = async_stream::stream! {
        let (url, headers, body) = crate::ai::anthropic::build_anthropic_stream_request(
            &token, is_setup_token, &model, max_tokens, &system, &messages,
        );

        let client = reqwest::Client::new();
        let mut req = client.post(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let res = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                yield StreamChunk::Error { message: format!("Network error: {}", e) };
                return;
            }
        };

        let status = res.status().as_u16();
        if status != 200 {
            let text = res.text().await.unwrap_or_default();
            yield StreamChunk::Error { message: format!("Anthropic API error ({}): {}", status, text) };
            return;
        }

        use futures::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();
        let mut cumulative_index: u32 = 0;
        let mut input_tokens: Option<u64> = None;
        let mut output_tokens: Option<u64> = None;
        let mut resolved_model = model.clone();
        let mut done_emitted = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    yield StreamChunk::Error { message: format!("Stream read error: {}", e) };
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // T-11.7-08: cap unbounded partial-frame growth.
            if buffer.len() > 1024 * 1024 {
                yield StreamChunk::Error { message: "SSE frame exceeded 1 MiB — aborting stream.".to_string() };
                return;
            }

            let (frames, remainder) = split_sse_frames(&buffer);
            buffer = remainder;

            for frame in frames {
                if frame.trim().is_empty() {
                    continue;
                }
                let (event, data) = parse_sse_frame(&frame);
                if data.is_empty() {
                    continue;
                }
                let json: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let event_type = event.clone().unwrap_or_else(|| json["type"].as_str().unwrap_or("").to_string());

                match event_type.as_str() {
                    "message_start" => {
                        if let Some(m) = json["message"]["model"].as_str() {
                            resolved_model = m.to_string();
                        }
                        if let Some(v) = json["message"]["usage"]["input_tokens"].as_u64() {
                            input_tokens = Some(v);
                        }
                    }
                    "content_block_delta" => {
                        if json["delta"]["type"] == "text_delta" {
                            if let Some(text) = json["delta"]["text"].as_str() {
                                cumulative_index += 1;
                                yield StreamChunk::Token { token: text.to_string(), cumulative_index };
                            }
                        }
                    }
                    "message_delta" => {
                        if let Some(v) = json["usage"]["input_tokens"].as_u64() {
                            input_tokens = Some(v);
                        }
                        if let Some(v) = json["usage"]["output_tokens"].as_u64() {
                            output_tokens = Some(v);
                        }
                    }
                    "message_stop" => {
                        done_emitted = true;
                        yield StreamChunk::Done { input_tokens, output_tokens, model: resolved_model.clone() };
                    }
                    "error" => {
                        let message = json["error"]["message"].as_str().unwrap_or("Anthropic stream error").to_string();
                        yield StreamChunk::Error { message };
                        return;
                    }
                    _ => {}
                }
            }
        }

        if !done_emitted {
            yield StreamChunk::Done { input_tokens, output_tokens, model: resolved_model };
        }
    };

    Box::pin(stream)
}

// ============================================================================
// OpenAI (Chat Completions) SSE streaming
// ============================================================================

/// Parse a complete (or partial) OpenAI Chat Completions SSE payload into
/// StreamChunk items. Pure function — no I/O — for unit testing.
///
/// Frame shape: `data: {json}\n\n`, terminated by `data: [DONE]\n\n`.
/// Emits Token for each `choices[0].delta.content` string. Emits Done when a
/// frame carries a top-level `usage` object (only present in the final chunk
/// when `stream_options.include_usage` is set).
pub fn parse_openai_sse_frames(input: &str) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let (frames, _remainder) = split_sse_frames(input);

    let mut cumulative_index: u32 = 0;
    let mut model = String::new();

    for frame in frames {
        if frame.trim().is_empty() {
            continue;
        }
        let (_event, data) = parse_sse_frame(&frame);
        if data.is_empty() {
            continue;
        }
        if data.trim() == "[DONE]" {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue, // unparseable frame — discard (T-11.7-08)
        };

        if let Some(m) = json["model"].as_str() {
            model = m.to_string();
        }

        if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
            if !content.is_empty() {
                cumulative_index += 1;
                chunks.push(StreamChunk::Token {
                    token: content.to_string(),
                    cumulative_index,
                });
            }
        }

        if json.get("usage").is_some() && !json["usage"].is_null() {
            let input_tokens = json["usage"]["prompt_tokens"].as_u64();
            let output_tokens = json["usage"]["completion_tokens"].as_u64();
            chunks.push(StreamChunk::Done {
                input_tokens,
                output_tokens,
                model: model.clone(),
            });
        }
    }

    chunks
}

/// Stream a chat request to the OpenAI Chat Completions API.
///
/// **No 401 retry** — see module doc comment.
pub fn openai_chat_stream(
    token: String,
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<crate::ai::ServiceMessage>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>> {
    let stream = async_stream::stream! {
        let (url, headers, body) = crate::ai::openai::build_openai_stream_request(
            &token, &model, max_tokens, &system, &messages,
        );

        let client = reqwest::Client::new();
        let mut req = client.post(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let res = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                yield StreamChunk::Error { message: format!("Network error: {}", e) };
                return;
            }
        };

        let status = res.status().as_u16();
        if status != 200 {
            let text = res.text().await.unwrap_or_default();
            yield StreamChunk::Error { message: format!("OpenAI API error ({}): {}", status, text) };
            return;
        }

        use futures::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();
        let mut cumulative_index: u32 = 0;
        let mut resolved_model = model.clone();
        let mut done_emitted = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    yield StreamChunk::Error { message: format!("Stream read error: {}", e) };
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            if buffer.len() > 1024 * 1024 {
                yield StreamChunk::Error { message: "SSE frame exceeded 1 MiB — aborting stream.".to_string() };
                return;
            }

            let (frames, remainder) = split_sse_frames(&buffer);
            buffer = remainder;

            for frame in frames {
                if frame.trim().is_empty() {
                    continue;
                }
                let (_event, data) = parse_sse_frame(&frame);
                if data.is_empty() || data.trim() == "[DONE]" {
                    continue;
                }
                let json: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(m) = json["model"].as_str() {
                    resolved_model = m.to_string();
                }

                if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                    if !content.is_empty() {
                        cumulative_index += 1;
                        yield StreamChunk::Token { token: content.to_string(), cumulative_index };
                    }
                }

                if json.get("usage").is_some() && !json["usage"].is_null() {
                    done_emitted = true;
                    let input_tokens = json["usage"]["prompt_tokens"].as_u64();
                    let output_tokens = json["usage"]["completion_tokens"].as_u64();
                    yield StreamChunk::Done { input_tokens, output_tokens, model: resolved_model.clone() };
                }
            }
        }

        if !done_emitted {
            yield StreamChunk::Done { input_tokens: None, output_tokens: None, model: resolved_model };
        }
    };

    Box::pin(stream)
}

// ============================================================================
// Codex (Responses API) SSE streaming
// ============================================================================

/// Parse a complete (or partial) Codex Responses API SSE payload into
/// StreamChunk items. Pure function — no I/O — for unit testing.
///
/// [ASSUMED] Event shape follows the OpenAI-published Responses API streaming
/// pattern: `response.output_text.delta`, `response.completed`, `response.failed`.
/// If actual wire bytes differ, adjust this parser — the change is isolated here.
pub fn parse_codex_sse_frames(input: &str) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let (frames, _remainder) = split_sse_frames(input);

    let mut cumulative_index: u32 = 0;
    let mut model = String::new();

    for frame in frames {
        if frame.trim().is_empty() {
            continue;
        }
        let (event, data) = parse_sse_frame(&frame);
        if data.is_empty() {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_type = event.clone().unwrap_or_else(|| json["type"].as_str().unwrap_or("").to_string());

        if let Some(m) = json["response"]["model"].as_str() {
            model = m.to_string();
        }

        match event_type.as_str() {
            "response.output_text.delta" => {
                if let Some(delta) = json["delta"].as_str() {
                    cumulative_index += 1;
                    chunks.push(StreamChunk::Token {
                        token: delta.to_string(),
                        cumulative_index,
                    });
                }
            }
            "response.completed" => {
                let input_tokens = json["response"]["usage"]["input_tokens"]
                    .as_u64()
                    .or_else(|| json["usage"]["input_tokens"].as_u64());
                let output_tokens = json["response"]["usage"]["output_tokens"]
                    .as_u64()
                    .or_else(|| json["usage"]["output_tokens"].as_u64());
                chunks.push(StreamChunk::Done {
                    input_tokens,
                    output_tokens,
                    model: model.clone(),
                });
            }
            "response.failed" => {
                let message = json["response"]["error"]["message"]
                    .as_str()
                    .or_else(|| json["error"]["message"].as_str())
                    .unwrap_or("Codex stream failed")
                    .to_string();
                chunks.push(StreamChunk::Error { message });
            }
            _ => {}
        }
    }

    chunks
}

/// Stream a chat request to the ChatGPT/Codex Responses API.
///
/// **No 401 retry** — see module doc comment.
pub fn codex_chat_stream(
    token: String,
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<crate::ai::ServiceMessage>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>> {
    let stream = async_stream::stream! {
        let (url, headers, body) = crate::ai::openai::build_codex_stream_request(
            &token, &model, max_tokens, &system, &messages,
        );

        let client = reqwest::Client::new();
        let mut req = client.post(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let res = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                yield StreamChunk::Error { message: format!("Network error: {}", e) };
                return;
            }
        };

        let status = res.status().as_u16();
        if status != 200 {
            let text = res.text().await.unwrap_or_default();
            yield StreamChunk::Error { message: format!("ChatGPT Codex error ({}): {}", status, text) };
            return;
        }

        use futures::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();
        let mut cumulative_index: u32 = 0;
        let mut resolved_model = model.clone();
        let mut done_emitted = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    yield StreamChunk::Error { message: format!("Stream read error: {}", e) };
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            if buffer.len() > 1024 * 1024 {
                yield StreamChunk::Error { message: "SSE frame exceeded 1 MiB — aborting stream.".to_string() };
                return;
            }

            let (frames, remainder) = split_sse_frames(&buffer);
            buffer = remainder;

            for frame in frames {
                if frame.trim().is_empty() {
                    continue;
                }
                let (event, data) = parse_sse_frame(&frame);
                if data.is_empty() {
                    continue;
                }
                let json: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let event_type = event.clone().unwrap_or_else(|| json["type"].as_str().unwrap_or("").to_string());

                if let Some(m) = json["response"]["model"].as_str() {
                    resolved_model = m.to_string();
                }

                match event_type.as_str() {
                    "response.output_text.delta" => {
                        if let Some(delta) = json["delta"].as_str() {
                            cumulative_index += 1;
                            yield StreamChunk::Token { token: delta.to_string(), cumulative_index };
                        }
                    }
                    "response.completed" => {
                        done_emitted = true;
                        let input_tokens = json["response"]["usage"]["input_tokens"].as_u64()
                            .or_else(|| json["usage"]["input_tokens"].as_u64());
                        let output_tokens = json["response"]["usage"]["output_tokens"].as_u64()
                            .or_else(|| json["usage"]["output_tokens"].as_u64());
                        yield StreamChunk::Done { input_tokens, output_tokens, model: resolved_model.clone() };
                    }
                    "response.failed" => {
                        let message = json["response"]["error"]["message"].as_str()
                            .or_else(|| json["error"]["message"].as_str())
                            .unwrap_or("Codex stream failed").to_string();
                        yield StreamChunk::Error { message };
                        return;
                    }
                    _ => {}
                }
            }
        }

        if !done_emitted {
            yield StreamChunk::Done { input_tokens: None, output_tokens: None, model: resolved_model };
        }
    };

    Box::pin(stream)
}

// ============================================================================
// Ollama NDJSON streaming
// ============================================================================

/// Parse a complete (or partial) Ollama `/api/chat` response body into
/// StreamChunk items. Pure function — no I/O — for unit testing.
///
/// Ollama's streaming wire format is newline-delimited JSON (NDJSON), NOT SSE:
/// each line is a standalone JSON object, no `data:` prefix, no blank-line
/// frame separator.
///
/// Also handles the non-streaming fallback: if the entire input is a single
/// JSON object (no trailing newline-delimited `done:true` marker mid-stream),
/// this still produces exactly one Token + one Done — Ollama disabling
/// streaming must not crash the parser (T-11.7 fallback requirement).
pub fn parse_ollama_ndjson_frames(input: &str) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let mut cumulative_index: u32 = 0;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // unparseable frame — discard (T-11.7-08)
        };

        let done = json["done"].as_bool().unwrap_or(false);
        let model = json["model"].as_str().unwrap_or("").to_string();

        if !done {
            if let Some(content) = json["message"]["content"].as_str() {
                if !content.is_empty() {
                    cumulative_index += 1;
                    chunks.push(StreamChunk::Token {
                        token: content.to_string(),
                        cumulative_index,
                    });
                }
            }
        } else {
            // Ollama non-streaming fallback: a single monolithic response
            // (stream disabled server-side) still carries message.content
            // alongside done:true. Emit the Token first, then Done.
            if let Some(content) = json["message"]["content"].as_str() {
                if !content.is_empty() {
                    cumulative_index += 1;
                    chunks.push(StreamChunk::Token {
                        token: content.to_string(),
                        cumulative_index,
                    });
                }
            }
            let input_tokens = json["prompt_eval_count"].as_u64();
            let output_tokens = json["eval_count"].as_u64();
            chunks.push(StreamChunk::Done {
                input_tokens,
                output_tokens,
                model,
            });
        }
    }

    chunks
}

/// Stream a chat request to a local Ollama `/api/chat` endpoint.
///
/// Wire format: NDJSON (not SSE). Falls back gracefully if the local Ollama
/// instance ignores `stream: true` and returns a single JSON object — see
/// `parse_ollama_ndjson_frames` doc comment.
///
/// **No 401 retry** — Ollama has no auth, so this is moot; kept for symmetry
/// with the other providers' doc comments.
pub fn ollama_chat_stream(
    base_url: String,
    model: String,
    system_prompt: String,
    messages: Vec<crate::ai::ServiceMessage>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>> {
    let stream = async_stream::stream! {
        let mut msgs = vec![serde_json::json!({"role": "system", "content": system_prompt})];
        for m in &messages {
            msgs.push(serde_json::json!({"role": m.role, "content": m.content}));
        }
        let body = serde_json::json!({"model": model, "messages": msgs, "stream": true});

        let client = reqwest::Client::new();
        let res = match client
            .post(format!("{}/api/chat", base_url))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                yield StreamChunk::Error { message: format!("Cannot reach Ollama at {}: {}", base_url, e) };
                return;
            }
        };

        let status = res.status().as_u16();
        if status != 200 {
            let text = res.text().await.unwrap_or_default();
            yield StreamChunk::Error { message: format!("Ollama error ({}): {}", status, text) };
            return;
        }

        use futures::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();
        let mut cumulative_index: u32 = 0;
        let mut resolved_model = model.clone();
        let mut done_emitted = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    yield StreamChunk::Error { message: format!("Stream read error: {}", e) };
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            if buffer.len() > 1024 * 1024 {
                yield StreamChunk::Error { message: "NDJSON line exceeded 1 MiB — aborting stream.".to_string() };
                return;
            }

            // NDJSON: split on newline, keep trailing partial line buffered.
            let mut lines: Vec<String> = buffer.split('\n').map(|s| s.to_string()).collect();
            let remainder = lines.pop().unwrap_or_default();
            buffer = remainder;

            for line in lines {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let json: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(m) = json["model"].as_str() {
                    resolved_model = m.to_string();
                }

                let done = json["done"].as_bool().unwrap_or(false);
                if let Some(content) = json["message"]["content"].as_str() {
                    if !content.is_empty() {
                        cumulative_index += 1;
                        yield StreamChunk::Token { token: content.to_string(), cumulative_index };
                    }
                }
                if done {
                    done_emitted = true;
                    let input_tokens = json["prompt_eval_count"].as_u64();
                    let output_tokens = json["eval_count"].as_u64();
                    yield StreamChunk::Done { input_tokens, output_tokens, model: resolved_model.clone() };
                }
            }
        }

        if !done_emitted {
            yield StreamChunk::Done { input_tokens: None, output_tokens: None, model: resolved_model };
        }
    };

    Box::pin(stream)
}

// ============================================================================
// Gemini SSE streaming
// ============================================================================

/// Parse a complete (or partial) Gemini `streamGenerateContent?alt=sse`
/// payload into StreamChunk items. Pure function — no I/O — for unit testing.
///
/// Frame shape: `data: {json}\n\n` where json =
/// `{"candidates":[{"content":{"parts":[{"text":"..."}]}}], "usageMetadata": {...}}`.
/// The `usageMetadata` field typically only appears on the final frame.
pub fn parse_gemini_sse_frames(input: &str) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let (frames, _remainder) = split_sse_frames(input);

    let mut cumulative_index: u32 = 0;
    let model = String::new();

    for frame in frames {
        if frame.trim().is_empty() {
            continue;
        }
        let (_event, data) = parse_sse_frame(&frame);
        if data.is_empty() {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue, // unparseable frame — discard (T-11.7-08)
        };

        if let Some(text) = json["candidates"][0]["content"]["parts"][0]["text"].as_str() {
            if !text.is_empty() {
                cumulative_index += 1;
                chunks.push(StreamChunk::Token {
                    token: text.to_string(),
                    cumulative_index,
                });
            }
        }

        if json.get("usageMetadata").is_some() && !json["usageMetadata"].is_null() {
            let input_tokens = json["usageMetadata"]["promptTokenCount"].as_u64();
            let output_tokens = json["usageMetadata"]["candidatesTokenCount"].as_u64();
            chunks.push(StreamChunk::Done {
                input_tokens,
                output_tokens,
                model: model.clone(),
            });
        }
    }

    chunks
}

/// Stream a chat request to the Gemini `streamGenerateContent` endpoint.
///
/// Uses `alt=sse` explicitly to get SSE frames instead of the default JSON
/// array chunk format. API-key auth appends `?key=...` to the URL; OAuth uses
/// a Bearer header with no query key.
///
/// If the stream ends without an explicit `usageMetadata` frame, emits Done
/// with `input_tokens: None, output_tokens: None` (no fabrication).
///
/// **No 401 retry** — see module doc comment.
pub fn gemini_chat_stream(
    credential: String,
    is_oauth: bool,
    model: String,
    max_tokens: u32,
    system_prompt: String,
    messages: Vec<crate::ai::ServiceMessage>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>> {
    let stream = async_stream::stream! {
        let url = if is_oauth {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                model
            )
        } else {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
                model, credential
            )
        };

        let client = reqwest::Client::new();
        let mut req = client.post(&url).header("content-type", "application/json");
        if is_oauth {
            req = req.header("Authorization", format!("Bearer {}", credential));
        }

        let contents: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let role = if m.role == "assistant" { "model" } else { "user" };
                serde_json::json!({"role": role, "parts": [{"text": m.content}]})
            })
            .collect();

        let body = serde_json::json!({
            "contents": contents,
            "systemInstruction": {"parts": [{"text": system_prompt}]},
            "generationConfig": {"maxOutputTokens": max_tokens},
        });

        let res = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                yield StreamChunk::Error { message: format!("Network error: {}", e) };
                return;
            }
        };

        let status = res.status().as_u16();
        if status != 200 {
            let text = res.text().await.unwrap_or_default();
            yield StreamChunk::Error { message: format!("Gemini API error ({}): {}", status, text) };
            return;
        }

        use futures::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();
        let mut cumulative_index: u32 = 0;
        let mut done_emitted = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    yield StreamChunk::Error { message: format!("Stream read error: {}", e) };
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            if buffer.len() > 1024 * 1024 {
                yield StreamChunk::Error { message: "SSE frame exceeded 1 MiB — aborting stream.".to_string() };
                return;
            }

            let (frames, remainder) = split_sse_frames(&buffer);
            buffer = remainder;

            for frame in frames {
                if frame.trim().is_empty() {
                    continue;
                }
                let (_event, data) = parse_sse_frame(&frame);
                if data.is_empty() {
                    continue;
                }
                let json: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(text) = json["candidates"][0]["content"]["parts"][0]["text"].as_str() {
                    if !text.is_empty() {
                        cumulative_index += 1;
                        yield StreamChunk::Token { token: text.to_string(), cumulative_index };
                    }
                }

                if json.get("usageMetadata").is_some() && !json["usageMetadata"].is_null() {
                    done_emitted = true;
                    let input_tokens = json["usageMetadata"]["promptTokenCount"].as_u64();
                    let output_tokens = json["usageMetadata"]["candidatesTokenCount"].as_u64();
                    yield StreamChunk::Done { input_tokens, output_tokens, model: model.clone() };
                }
            }
        }

        if !done_emitted {
            yield StreamChunk::Done { input_tokens: None, output_tokens: None, model: model.clone() };
        }
    };

    Box::pin(stream)
}

#[cfg(test)]
mod anthropic {
    use super::*;
    use crate::ai::anthropic::build_anthropic_stream_request;
    use crate::ai::ServiceMessage;

    fn sample_messages() -> Vec<ServiceMessage> {
        vec![ServiceMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }]
    }

    #[test]
    fn test_build_anthropic_stream_request_adds_stream_flag() {
        let msgs = sample_messages();
        let (url, headers, body) = build_anthropic_stream_request(
            "setup-token-abc",
            true,
            "claude-haiku-4-5",
            100,
            "system prompt",
            &msgs,
        );

        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(body["stream"], true);

        // Unchanged headers from build_anthropic_request
        let auth = headers.get("authorization").unwrap().to_str().unwrap();
        assert_eq!(auth, "Bearer setup-token-abc");
        let beta = headers.get("anthropic-beta").unwrap().to_str().unwrap();
        assert_eq!(beta, "oauth-2025-04-20");
    }

    #[test]
    fn test_parse_anthropic_sse_message_delta() {
        let sse = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-haiku-4-5\",\"usage\":{\"input_tokens\":5}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo \"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n",
        );

        let chunks = parse_anthropic_sse_frames(sse);
        let tokens: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Token { .. }))
            .collect();
        assert_eq!(tokens.len(), 3, "expected 3 Token chunks, got: {:?}", chunks);

        let mut concatenated = String::new();
        for (i, c) in tokens.iter().enumerate() {
            if let StreamChunk::Token { token, cumulative_index } = c {
                assert_eq!(*cumulative_index, (i + 1) as u32);
                concatenated.push_str(token);
            }
        }
        assert_eq!(concatenated, "Hello world");
    }

    #[test]
    fn test_parse_anthropic_sse_final_usage() {
        let sse = concat!(
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"input_tokens\":10,\"output_tokens\":25}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let chunks = parse_anthropic_sse_frames(sse);
        let done = chunks
            .iter()
            .find(|c| matches!(c, StreamChunk::Done { .. }))
            .expect("expected a Done chunk");
        if let StreamChunk::Done { input_tokens, output_tokens, .. } = done {
            assert_eq!(*input_tokens, Some(10));
            assert_eq!(*output_tokens, Some(25));
        }
    }

    #[test]
    fn test_parse_anthropic_sse_error_event() {
        let sse = concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        );

        let chunks = parse_anthropic_sse_frames(sse);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            StreamChunk::Error { message } => assert_eq!(message, "Overloaded"),
            other => panic!("expected Error chunk, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod openai {
    use super::*;

    #[test]
    fn test_parse_openai_sse_text_delta() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
            "data: [DONE]\n\n",
        );

        let chunks = parse_openai_sse_frames(sse);
        let tokens: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Token { .. }))
            .collect();
        assert_eq!(tokens.len(), 2, "got: {:?}", chunks);

        let mut concatenated = String::new();
        for (i, c) in tokens.iter().enumerate() {
            if let StreamChunk::Token { token, cumulative_index } = c {
                assert_eq!(*cumulative_index, (i + 1) as u32);
                concatenated.push_str(token);
            }
        }
        assert_eq!(concatenated, "Hello world");
    }

    #[test]
    fn test_parse_openai_sse_final_usage() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":25,\"total_tokens\":35}}\n\n",
            "data: [DONE]\n\n",
        );

        let chunks = parse_openai_sse_frames(sse);
        let done = chunks
            .iter()
            .find(|c| matches!(c, StreamChunk::Done { .. }))
            .expect("expected a Done chunk");
        if let StreamChunk::Done { input_tokens, output_tokens, .. } = done {
            assert_eq!(*input_tokens, Some(10));
            assert_eq!(*output_tokens, Some(25));
        }
    }
}

#[cfg(test)]
mod codex {
    use super::*;

    #[test]
    fn test_parse_codex_sse_response_delta() {
        let sse = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\" world\"}\n\n",
        );

        let chunks = parse_codex_sse_frames(sse);
        let tokens: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Token { .. }))
            .collect();
        assert_eq!(tokens.len(), 2, "got: {:?}", chunks);

        let mut concatenated = String::new();
        for (i, c) in tokens.iter().enumerate() {
            if let StreamChunk::Token { token, cumulative_index } = c {
                assert_eq!(*cumulative_index, (i + 1) as u32);
                concatenated.push_str(token);
            }
        }
        assert_eq!(concatenated, "Hello world");
    }

    #[test]
    fn test_parse_codex_sse_completed_with_usage() {
        let sse = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":25}}}\n\n",
        );

        let chunks = parse_codex_sse_frames(sse);
        let done = chunks
            .iter()
            .find(|c| matches!(c, StreamChunk::Done { .. }))
            .expect("expected a Done chunk");
        if let StreamChunk::Done { input_tokens, output_tokens, model } = done {
            assert_eq!(*input_tokens, Some(10));
            assert_eq!(*output_tokens, Some(25));
            assert_eq!(model, "gpt-5");
        }
    }
}

#[cfg(test)]
mod ollama {
    use super::*;

    #[test]
    fn test_parse_ollama_ndjson_chunks() {
        let ndjson = concat!(
            "{\"message\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"lo\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":10,\"eval_count\":25}\n",
        );

        let chunks = parse_ollama_ndjson_frames(ndjson);
        let tokens: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Token { .. }))
            .collect();
        assert_eq!(tokens.len(), 2, "expected 2 Token chunks, got: {:?}", chunks);

        let mut concatenated = String::new();
        for (i, c) in tokens.iter().enumerate() {
            if let StreamChunk::Token { token, cumulative_index } = c {
                assert_eq!(*cumulative_index, (i + 1) as u32);
                concatenated.push_str(token);
            }
        }
        assert_eq!(concatenated, "Hello");

        let done = chunks
            .iter()
            .find(|c| matches!(c, StreamChunk::Done { .. }))
            .expect("expected a Done chunk");
        if let StreamChunk::Done { input_tokens, output_tokens, .. } = done {
            assert_eq!(*input_tokens, Some(10));
            assert_eq!(*output_tokens, Some(25));
        }
    }

    #[test]
    fn test_ollama_fallback_on_non_streaming() {
        // Ollama server refused streaming — the entire body is one JSON object
        // with done:true and message.content set (monolithic response).
        let single_object = concat!(
            "{\"model\":\"llama3\",\"message\":{\"role\":\"assistant\",\"content\":\"Full response text\"},",
            "\"done\":true,\"prompt_eval_count\":12,\"eval_count\":8}\n"
        );

        let chunks = parse_ollama_ndjson_frames(single_object);

        let tokens: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Token { .. }))
            .collect();
        assert_eq!(tokens.len(), 1, "expected exactly 1 Token chunk, got: {:?}", chunks);
        if let StreamChunk::Token { token, cumulative_index } = tokens[0] {
            assert_eq!(token, "Full response text");
            assert_eq!(*cumulative_index, 1);
        }

        let dones: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Done { .. }))
            .collect();
        assert_eq!(dones.len(), 1, "expected exactly 1 Done chunk");
        if let StreamChunk::Done { input_tokens, output_tokens, model } = dones[0] {
            assert_eq!(*input_tokens, Some(12));
            assert_eq!(*output_tokens, Some(8));
            assert_eq!(model, "llama3");
        }
    }
}

#[cfg(test)]
mod gemini {
    use super::*;

    #[test]
    fn test_parse_gemini_sse_stream() {
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]}}],",
            "\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":25}}\n\n",
        );

        let chunks = parse_gemini_sse_frames(sse);

        let tokens: Vec<&StreamChunk> = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Token { .. }))
            .collect();
        assert_eq!(tokens.len(), 2, "got: {:?}", chunks);

        let mut concatenated = String::new();
        for (i, c) in tokens.iter().enumerate() {
            if let StreamChunk::Token { token, cumulative_index } = c {
                assert_eq!(*cumulative_index, (i + 1) as u32);
                concatenated.push_str(token);
            }
        }
        assert_eq!(concatenated, "Hello world");

        let done = chunks
            .iter()
            .find(|c| matches!(c, StreamChunk::Done { .. }))
            .expect("expected a Done chunk");
        if let StreamChunk::Done { input_tokens, output_tokens, .. } = done {
            assert_eq!(*input_tokens, Some(10));
            assert_eq!(*output_tokens, Some(25));
        }
    }

    #[test]
    fn test_parse_gemini_sse_no_usage_frame_yields_none() {
        // If a stream ends without an explicit usage frame, no Done chunk is
        // synthesized by the pure parser (the async wrapper handles the
        // stream-end fallback separately). This test documents that the
        // pure parser only emits Done when usageMetadata is present.
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hi\"}]}}]}\n\n";
        let chunks = parse_gemini_sse_frames(sse);
        assert!(
            !chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })),
            "no Done chunk expected without usageMetadata frame, got: {:?}",
            chunks
        );
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use crate::auth::AuthState;

    #[tokio::test]
    async fn test_ai_request_stream_no_credentials_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthState::new(&dir.path().to_path_buf());
        let request = AIServiceRequest {
            system_prompt: String::new(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            response_format: None,
            model_override: None,
        };
        let result = ai_request_stream(&auth, request).await;
        match result {
            Err(e) => assert!(e.contains("No AI provider configured"), "got: {}", e),
            Ok(_) => panic!("expected Err(\"No AI provider configured...\"), got Ok"),
        }
    }
}
