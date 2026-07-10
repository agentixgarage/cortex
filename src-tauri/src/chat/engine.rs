//! RAG orchestration for Chat with Your Docs (Phase 11.7, Plan 05).
//!
//! `ChatEngine::answer` runs the full per-query pipeline:
//! 1. Embed the query via `EmbeddingService`.
//! 2. HNSW top-8 docs from the `documents_384` collection, honoring optional
//!    `SearchFilters` (D-04 filter passthrough).
//! 3. Chunk each candidate doc's excerpt (500 chars / 50-char overlap), embed
//!    each chunk, keep the top-3 per doc by cosine similarity to the query.
//! 4. Rerank the resulting <=24 candidates down to the top-12 (D-01).
//! 5. If the best chunk score is below the 0.35 cosine floor (D-03), return a
//!    canned "not found" answer without calling the LLM.
//! 6. Otherwise build the RAG prompt (D-06/D-07) and stream the LLM response,
//!    forwarding tokens to the Tauri event bus and persisting the assistant
//!    message + citations on completion.
//!
//! The pure helpers below (`chunk_text`, `cosine_sim`, `rerank_chunks`,
//! `build_rag_prompt`, `answer_or_canned`) are unit-tested in isolation —
//! no I/O, no async — so the RAG math can be verified without a live model
//! or network access.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Emitter;
use tokio::sync::Mutex;

use crate::auth::AuthState;
use crate::chat::session_store::ChatSessionStore;
use crate::engine::CortexEngine;
use crate::graph::entity_store::EntityStore;
use crate::pipeline::embedder::EmbeddingService;
use crate::types::{
    ChatMessage, ChatRole, ChatStreamCompletePayload, ChatStreamErrorPayload,
    ChatStreamSuggestionsPayload, ChatStreamTokenPayload, Citation, SearchFilters,
};

/// Cosine-similarity floor below which the engine short-circuits to a canned
/// "not found" answer without calling the LLM (D-03).
const COSINE_FLOOR: f32 = 0.20;

/// The exact system prompt text from D-06 (11.7-CONTEXT.md). Preserve line
/// breaks verbatim — this is sent as-is to every provider.
pub const RAG_SYSTEM_PROMPT: &str = "You are a helpful assistant answering questions about the user's personal documents. Be precise, concrete, and grounded.\n\nGeneral rules:\n- Answer using the numbered document excerpts below. Cite as [1], [2] matching the excerpts.\n- Reasonable inference IS allowed: use context clues (document titles, shared surnames, matching addresses, matching dates of birth, explicit relation words) to draw conclusions the document implies. When you infer rather than quote, say so briefly (\"appears to be…\", \"based on shared surname…\").\n- Extract concrete data (numbers, dates, addresses, amounts, names) directly from the excerpts. Read carefully — receipts, deeds, IDs, invoices, statements each contain distinct fields.\n- Never cite a source not in the list. Never invent numbers, dates, IDs, or names.\n- If the info is genuinely absent from all excerpts, say so explicitly. Do not pad the answer with unrelated content just because a document was retrieved.\n\nQuery-type handling:\n- Aggregate queries (\"how much total\", \"how many\", \"list all\"): iterate every relevant excerpt, sum/enumerate, then report per-doc breakdown + total. Preserve the exact currency symbol as it appears in the source (₹, $, £, €, ¥, etc.). Do not convert currencies or drop the symbol. If the amounts span multiple currencies, report each subtotal separately — never sum across currencies without saying so.\n- Year-specific queries (\"in 2020\", \"last year\", \"FY 2023-24\"): only include excerpts whose date/filename/content matches the asked year. Ignore other years even if they were retrieved.\n- Entity-scoped queries (\"for X vs Y\", \"in city Z\", \"for asset X\", \"about person Y\"): only include excerpts explicitly tied to the named entity. If an excerpt could be about either or neither, say so. Entities can be people, organizations, properties, vehicles, projects, accounts, or anything else that appears in the corpus.\n- Multi-jurisdiction queries: users may hold assets, documents, or transactions across multiple countries. Never assume a single jurisdiction. Preserve country-specific fields as-is (US SSN, India Aadhaar/PAN, UK NI, EU IBAN, etc.) and country-specific formats (currency symbols, date formats).\n- Follow-up queries (\"of that\", \"how much of that for X\", \"what about last year\"): treat the prior turn as context. If the current query is a filter on the previous answer's scope, apply that filter.\n- Comparison queries (\"which cost more\"): pull the compared numbers, state them, then answer.\n\nGrounding discipline:\n- If asked about a specific year and no retrieved excerpt is from that year, say \"I don't have documents for {year} in the retrieved excerpts\" rather than guessing.\n- If asked about an entity not present in any excerpt, say \"I don't have documents about {entity}\".\n- Never confuse document types: an income-tax return is not a property-tax receipt; an insurance policy is not an invoice.";

/// Canned answer returned when no chunk clears the cosine floor (D-03).
const NO_MATCH_ANSWER: &str = "I couldn't find anything relevant in your library.";

/// Build the effective system prompt: `RAG_SYSTEM_PROMPT` plus an optional
/// "Known context about the user" block derived from their profile
/// (v1.2 #2). Empty profile → prompt is returned unchanged (byte-identical
/// to `RAG_SYSTEM_PROMPT`), so existing tests/behavior for users who skip
/// onboarding are unaffected.
///
/// This directly fixes cases like "generate my family tree" — without a
/// known-family hint the LLM sees documents with matching surnames and
/// correctly refuses to guess relationships; with the user-confirmed
/// relations from onboarding, it can state them directly instead of
/// hedging or refusing.
pub fn build_system_prompt(profile: &crate::types::UserProfile) -> String {
    let has_content = !profile.display_name.trim().is_empty()
        || !profile.aliases.is_empty()
        || !profile.family_members.is_empty()
        || !profile.countries.is_empty()
        || !profile.currencies.is_empty();

    if !has_content {
        return RAG_SYSTEM_PROMPT.to_string();
    }

    let mut block = String::from("\n\nKnown context about the user (confirmed by the user, not inferred — trust this over document-only guesses):");
    if !profile.display_name.trim().is_empty() {
        block.push_str(&format!("\n- The user's name is {}.", profile.display_name.trim()));
    }
    if !profile.aliases.is_empty() {
        block.push_str(&format!(
            "\n- The user is also known by: {}.",
            profile.aliases.join(", ")
        ));
    }
    if !profile.family_members.is_empty() {
        let rels: Vec<String> = profile
            .family_members
            .iter()
            .map(|f| format!("{} ({})", f.name, f.relation))
            .collect();
        block.push_str(&format!("\n- Family members: {}.", rels.join(", ")));
    }
    if !profile.countries.is_empty() {
        block.push_str(&format!(
            "\n- The user has documents/assets in: {}.",
            profile.countries.join(", ")
        ));
    }
    if !profile.currencies.is_empty() {
        block.push_str(&format!(
            "\n- Currencies commonly used in the user's documents: {}.",
            profile.currencies.join(", ")
        ));
    }

    format!("{}{}", RAG_SYSTEM_PROMPT, block)
}

/// A single chunk of a document's text, with char offsets so citations can
/// reference back to the exact span (D-02: 500 chars / 50-char overlap).
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub text: String,
    pub start: u32,
    pub end: u32,
}

/// Sliding-window chunker over CHARS (not bytes) with overlap.
///
/// Given `text`, produces windows of `size` chars advancing by `size -
/// overlap` chars each step. The final chunk may be shorter than `size`.
/// A text shorter than `size` produces exactly one chunk spanning the whole
/// input.
pub fn chunk_text(text: &str, size: usize, overlap: usize) -> Vec<Chunk> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    if len == 0 {
        return Vec::new();
    }

    if len <= size {
        return vec![Chunk {
            text: text.to_string(),
            start: 0,
            end: len as u32,
        }];
    }

    let stride = size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < len {
        let end = (start + size).min(len);
        let slice: String = chars[start..end].iter().collect();
        chunks.push(Chunk {
            text: slice,
            start: start as u32,
            end: end as u32,
        });
        if end == len {
            break;
        }
        start += stride;
    }

    chunks
}

/// Cosine similarity between two vectors. Returns 0.0 on zero norm (avoids
/// division by zero / NaN propagation).
pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Sort `candidates` descending by score and keep the top `top_k`.
pub fn rerank_chunks(
    mut candidates: Vec<(String, Chunk, f32)>,
    top_k: usize,
) -> Vec<(String, Chunk, f32)> {
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    candidates.truncate(top_k);
    candidates
}

/// Build the D-07 user-message shape:
///
/// ```text
/// Documents in your library:
/// [1] {title_1}: {chunk_1_text}
/// [2] {title_2}: {chunk_2_text}
/// ...
///
/// Question: {user_query}
/// ```
///
/// `system` is accepted for symmetry with the RAG pipeline's call site but is
/// NOT embedded in the returned string — it is sent as a separate
/// `system_prompt` field on `AIServiceRequest`. Kept as a parameter so the
/// function signature documents the full prompt-assembly contract.
pub fn build_rag_prompt(_system: &str, numbered_docs: &[(u32, &str, &str)], query: &str) -> String {
    let mut out = String::from("Documents in your library:\n");
    for (index, title, chunk_text) in numbered_docs {
        out.push_str(&format!("[{}] {}: {}\n", index, title, chunk_text));
    }
    out.push('\n');
    out.push_str(&format!("Question: {}", query));
    out
}

/// Returns the canned "not found" answer when `best_score` is below the
/// cosine floor (D-03); `None` when the score clears the floor and the LLM
/// should be called.
pub fn answer_or_canned(best_score: f32) -> Option<&'static str> {
    if best_score < COSINE_FLOOR {
        Some(NO_MATCH_ANSWER)
    } else {
        None
    }
}

/// Facade for the RAG pipeline. Stateless — all dependencies are passed into
/// `answer`.
pub struct ChatEngine;

impl ChatEngine {
    /// Runs the full RAG pipeline for a single query and persists the
    /// assistant's response (and citations, if any) to `chat_store`.
    ///
    /// See module doc comment for the full step-by-step description.
    #[allow(clippy::too_many_arguments)]
    pub async fn answer(
        app: tauri::AppHandle,
        auth: Arc<AuthState>,
        engine: Arc<Mutex<CortexEngine>>,
        embedding_service: Arc<EmbeddingService>,
        entity_store: Arc<std::sync::Mutex<EntityStore>>,
        chat_store: Arc<Mutex<ChatSessionStore>>,
        user_profile: Arc<Mutex<crate::types::UserProfile>>,
        app_data_dir: PathBuf,
        session_id: String,
        _user_message_id: String,
        assistant_message_id: String,
        query: String,
        filters: Option<SearchFilters>,
    ) -> Result<(), String> {
        // ── Load recent conversation history for context-aware retrieval + LLM ──
        // Last N turns keeps follow-ups grounded ("but you cited property tax…")
        // without blowing up prompt size.
        let history: Vec<ChatMessage> = {
            let store = chat_store.lock().await;
            store
                .get(&session_id)
                .map(|s| {
                    let msgs = &s.messages;
                    let take = msgs.len().saturating_sub(1).min(6); // exclude current user turn just appended
                    msgs.iter().rev().skip(1).take(take).cloned().collect::<Vec<_>>()
                        .into_iter().rev().collect()
                })
                .unwrap_or_default()
        };

        // Augmented retrieval query = last user turns + current query.
        // Follow-ups like "how much of that for X?" gain the missing subject.
        let augmented_query = Self::build_retrieval_query(&history, &query);

        // ── Retrieval (spawn_blocking: std::sync::Mutex guards must not cross .await) ──
        let retrieval_query = augmented_query.clone();
        let retrieval = tokio::task::spawn_blocking(move || {
            Self::retrieve_and_rerank(
                &retrieval_query,
                filters.as_ref(),
                &engine,
                &embedding_service,
                &entity_store,
            )
        })
        .await
        .map_err(|e| format!("retrieval task panicked: {}", e))??;

        let best_score = retrieval
            .iter()
            .map(|(_, _, _, score)| *score)
            .fold(f32::MIN, f32::max);
        let best_score = if retrieval.is_empty() { 0.0 } else { best_score };

        // ── Below-floor branch (D-03): canned answer, no LLM call ──
        if let Some(canned) = answer_or_canned(best_score) {
            let _ = app.emit(
                "chat-stream-token",
                ChatStreamTokenPayload {
                    session_id: session_id.clone(),
                    message_id: assistant_message_id.clone(),
                    token: canned.to_string(),
                    cumulative_index: 1,
                },
            );
            let _ = app.emit(
                "chat-stream-complete",
                ChatStreamCompletePayload {
                    session_id: session_id.clone(),
                    message_id: assistant_message_id.clone(),
                    citations: vec![],
                    input_tokens: None,
                    output_tokens: None,
                },
            );

            Self::persist_assistant_message(
                &chat_store,
                &app_data_dir,
                &session_id,
                &assistant_message_id,
                canned,
                None,
            )
            .await;

            return Ok(());
        }

        // Dedup: cap at 3 chunks per doc — one doc-spam bug pattern is a
        // single 5MB PDF winning all 5 top slots and drowning other candidates.
        let mut per_doc_count: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let retrieval_dedup: Vec<_> = retrieval
            .iter()
            .filter(|(doc_id, _, _, _)| {
                let c = per_doc_count.entry(doc_id.clone()).or_insert(0);
                if *c >= 3 { return false; }
                *c += 1;
                true
            })
            .collect();

        // ── Build citations + numbered docs for the prompt ──
        let mut citations: Vec<Citation> = Vec::new();
        let mut numbered_docs: Vec<(u32, String, String)> = Vec::new();
        for (index, (doc_id, doc_title, chunk, _score)) in retrieval_dedup.iter().enumerate() {
            let doc_index = (index + 1) as u32;
            citations.push(Citation {
                index: doc_index,
                doc_id: doc_id.to_string(),
                doc_title: doc_title.to_string(),
                chunk_start: chunk.start,
                chunk_end: chunk.end,
            });
            numbered_docs.push((doc_index, doc_title.to_string(), chunk.text.clone()));
        }

        let numbered_docs_refs: Vec<(u32, &str, &str)> = numbered_docs
            .iter()
            .map(|(i, t, c)| (*i, t.as_str(), c.as_str()))
            .collect();

        let system_prompt = {
            let profile = user_profile.lock().await;
            build_system_prompt(&profile)
        };

        let prompt = build_rag_prompt(&system_prompt, &numbered_docs_refs, &query);

        // Build message list with recent history so LLM sees the conversation
        // arc, not just the current query in isolation. History is user↔assistant
        // pairs from earlier turns; assistants' prior citation lists are dropped
        // to save tokens — the doc excerpts for THIS turn are already numbered.
        let mut messages: Vec<crate::ai::ServiceMessage> = history
            .iter()
            .map(|m| crate::ai::ServiceMessage {
                role: match m.role {
                    ChatRole::User => "user".to_string(),
                    ChatRole::Assistant => "assistant".to_string(),
                },
                content: m.content.clone(),
            })
            .collect();
        messages.push(crate::ai::ServiceMessage {
            role: "user".to_string(),
            content: prompt,
        });

        let request = crate::ai::AIServiceRequest {
            system_prompt: system_prompt.clone(),
            messages,
            max_tokens: Some(4096),
            temperature: None,
            response_format: None,
            model_override: None,
        };

        let mut stream = match crate::ai::ai_request_stream(&auth, request).await {
            Ok(s) => s,
            Err(message) => {
                let _ = app.emit(
                    "chat-stream-error",
                    ChatStreamErrorPayload {
                        session_id: session_id.clone(),
                        message_id: assistant_message_id.clone(),
                        error: message.clone(),
                    },
                );
                Self::persist_assistant_message(
                    &chat_store,
                    &app_data_dir,
                    &session_id,
                    &assistant_message_id,
                    &message,
                    None,
                )
                .await;
                return Err(message);
            }
        };

        use futures::StreamExt;
        let mut content_buffer = String::new();

        while let Some(chunk) = stream.next().await {
            match chunk {
                crate::ai::StreamChunk::Token {
                    token,
                    cumulative_index,
                } => {
                    content_buffer.push_str(&token);
                    let _ = app.emit(
                        "chat-stream-token",
                        ChatStreamTokenPayload {
                            session_id: session_id.clone(),
                            message_id: assistant_message_id.clone(),
                            token,
                            cumulative_index,
                        },
                    );
                }
                crate::ai::StreamChunk::Done {
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    let _ = app.emit(
                        "chat-stream-complete",
                        ChatStreamCompletePayload {
                            session_id: session_id.clone(),
                            message_id: assistant_message_id.clone(),
                            citations: citations.clone(),
                            input_tokens,
                            output_tokens,
                        },
                    );
                    Self::persist_assistant_message(
                        &chat_store,
                        &app_data_dir,
                        &session_id,
                        &assistant_message_id,
                        &content_buffer,
                        Some(citations.clone()),
                    )
                    .await;
                    let suggestions = Self::generate_suggestions(&query, &citations);
                    if !suggestions.is_empty() {
                        let _ = app.emit(
                            "chat-stream-suggestions",
                            ChatStreamSuggestionsPayload {
                                session_id: session_id.clone(),
                                message_id: assistant_message_id.clone(),
                                suggestions,
                            },
                        );
                    }
                    return Ok(());
                }
                crate::ai::StreamChunk::Error { message } => {
                    let _ = app.emit(
                        "chat-stream-error",
                        ChatStreamErrorPayload {
                            session_id: session_id.clone(),
                            message_id: assistant_message_id.clone(),
                            error: message.clone(),
                        },
                    );
                    // Persist a truncated assistant message with the error so
                    // the user can see what failed after reload.
                    Self::persist_assistant_message(
                        &chat_store,
                        &app_data_dir,
                        &session_id,
                        &assistant_message_id,
                        &message,
                        None,
                    )
                    .await;
                    return Err(message);
                }
            }
        }

        // Stream ended without an explicit Done — treat as a soft success so
        // the user still sees whatever content arrived.
        let _ = app.emit(
            "chat-stream-complete",
            ChatStreamCompletePayload {
                session_id: session_id.clone(),
                message_id: assistant_message_id.clone(),
                citations: citations.clone(),
                input_tokens: None,
                output_tokens: None,
            },
        );
        Self::persist_assistant_message(
            &chat_store,
            &app_data_dir,
            &session_id,
            &assistant_message_id,
            &content_buffer,
            Some(citations.clone()),
        )
        .await;
        let suggestions = Self::generate_suggestions(&query, &citations);
        if !suggestions.is_empty() {
            let _ = app.emit(
                "chat-stream-suggestions",
                ChatStreamSuggestionsPayload {
                    session_id: session_id.clone(),
                    message_id: assistant_message_id.clone(),
                    suggestions,
                },
            );
        }

        Ok(())
    }

    /// Build an augmented query for retrieval that carries recent-turn context.
    /// Follow-ups like "how much of that for X?" gain the subject from prior
    /// turns; otherwise "of that for X" alone embeds to the wrong docs.
    ///
    /// Strategy: concatenate the last 2 user turns + last assistant answer +
    /// current query, all trimmed. Cheap, deterministic, no LLM call.
    fn build_retrieval_query(history: &[ChatMessage], current: &str) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Take last 2 user turns + last assistant answer for context.
        let user_turns: Vec<&ChatMessage> = history
            .iter()
            .filter(|m| m.role == ChatRole::User)
            .collect();
        let start = user_turns.len().saturating_sub(2);
        for m in &user_turns[start..] {
            parts.push(m.content.trim().to_string());
        }

        if let Some(last_a) = history.iter().rev().find(|m| m.role == ChatRole::Assistant) {
            // Keep only first 400 chars of last assistant answer to avoid
            // drowning the query embedding.
            let a: String = last_a.content.chars().take(400).collect();
            parts.push(a);
        }

        parts.push(current.trim().to_string());

        parts.join(" ")
    }

    /// Generate 0-3 follow-up question suggestions from the just-completed
    /// turn. Deterministic (template-based, no LLM call) — keeps suggestions
    /// free of extra latency/rate-limit risk. Generic across any domain:
    /// looks at years present in citation titles, distinct cited docs, and
    /// whether the query looked like an aggregate ("total", "how much",
    /// "how many") to decide which templates apply.
    ///
    /// [v1.2 #1: Chat suggestions]
    fn generate_suggestions(query: &str, citations: &[Citation]) -> Vec<String> {
        let mut suggestions: Vec<String> = Vec::new();
        let q_lower = query.to_lowercase();

        let mut years: Vec<String> = Vec::new();
        for c in citations {
            for y in Self::find_years(&c.doc_title) {
                if !years.contains(&y) {
                    years.push(y);
                }
            }
        }
        years.sort();

        let is_aggregate = q_lower.contains("total")
            || q_lower.contains("how much")
            || q_lower.contains("how many")
            || q_lower.contains("sum");

        // Template 1: multi-year citation set + aggregate query → offer breakdown.
        if is_aggregate && years.len() > 1 {
            suggestions.push("Can you break that down by year?".to_string());
        }

        // Template 2: multiple distinct docs cited → offer to see the list.
        let distinct_docs: std::collections::HashSet<&str> =
            citations.iter().map(|c| c.doc_title.as_str()).collect();
        if distinct_docs.len() > 1 {
            suggestions.push("Which documents did you use for this answer?".to_string());
        }

        // Template 3: aggregate query but query doesn't already scope a year → offer year filter.
        if is_aggregate && Self::find_years(&q_lower).is_empty() && !years.is_empty() {
            suggestions.push(format!(
                "What about just {}?",
                years.last().cloned().unwrap_or_default()
            ));
        }

        // Fallback templates if nothing else triggered — keep it generically useful.
        if suggestions.is_empty() && !citations.is_empty() {
            suggestions.push("Can you show me more detail on this?".to_string());
        }

        suggestions.truncate(3);
        suggestions
    }

    /// Find known canonical entities whose name (or an alias) appears in the
    /// query. Case-insensitive substring match. Returns entity IDs. Used for
    /// pre-filtering docs to those mentioning the same entity — massively
    /// improves accuracy on "for X vs Y" or "of that, how much for X" queries.
    fn entities_mentioned_in_query(
        query: &str,
        entity_store: &Arc<std::sync::Mutex<EntityStore>>,
    ) -> Vec<String> {
        let store = match entity_store.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let q_lower = query.to_lowercase();
        let mut ids: Vec<String> = Vec::new();
        for (id, ent) in store.canonicals.iter() {
            let name_lower = ent.canonical_name.to_lowercase();
            let hit_name = name_lower.len() >= 3 && q_lower.contains(&name_lower);
            let hit_alias = ent.aliases.iter().any(|a| {
                let al = a.to_lowercase();
                al.len() >= 3 && q_lower.contains(&al)
            });
            if hit_name || hit_alias {
                ids.push(id.clone());
            }
        }
        ids
    }

    /// Find 4-digit years (19xx / 20xx) anywhere in `s` by scanning maximal
    /// runs of ASCII digits — NOT regex `\b`, which fails to match a year
    /// preceded/followed by `_` (underscore is a word char, so "Receipt_2020"
    /// has no `\b` before "2020"). Filenames like `Prop_Tax_FY2020-21.pdf`
    /// are exactly the case this must handle correctly.
    ///
    /// A digit run longer than 4 (e.g. an 8-digit reference number) does NOT
    /// yield a false year — only EXACT 4-digit runs starting with 19/20 count.
    fn find_years(s: &str) -> Vec<String> {
        let mut years: Vec<String> = Vec::new();
        let mut run_start: Option<usize> = None;
        let bytes = s.as_bytes();

        let mut flush = |start: Option<usize>, end: usize, years: &mut Vec<String>| {
            if let Some(st) = start {
                let run = &s[st..end];
                if run.len() == 4 && (run.starts_with("19") || run.starts_with("20")) && !years.contains(&run.to_string()) {
                    years.push(run.to_string());
                }
            }
        };

        for (i, b) in bytes.iter().enumerate() {
            if b.is_ascii_digit() {
                if run_start.is_none() {
                    run_start = Some(i);
                }
            } else {
                flush(run_start, i, &mut years);
                run_start = None;
            }
        }
        flush(run_start, s.len(), &mut years);

        years
    }

    /// Extract 4-digit years (19xx / 20xx) from a query string. Used to
    /// pre-filter documents by year when the user asks about a specific year.
    /// Also recognizes `FY 2024-25` style (captures the start year).
    fn extract_years(query: &str) -> Vec<String> {
        let mut years = Self::find_years(query);
        let fy = regex::Regex::new(r"(?i)\bFY\s*(\d{4})[-\x{2013}\x{2014}](\d{2,4})\b").expect("fy regex");
        for cap in fy.captures_iter(query) {
            if let Some(start_year) = cap.get(1) {
                let y = start_year.as_str().to_string();
                if !years.contains(&y) {
                    years.push(y);
                }
            }
        }
        years
    }

    /// Blocking retrieval segment: embed query, HNSW top-K, chunk + embed +
    /// score, rerank to top-N. Runs inside `spawn_blocking` so the
    /// std::sync::Mutex guards on `engine`/`entity_store` never cross an
    /// `.await`.
    #[allow(clippy::type_complexity)]
    fn retrieve_and_rerank(
        query: &str,
        filters: Option<&SearchFilters>,
        engine: &Arc<Mutex<CortexEngine>>,
        embedding_service: &Arc<EmbeddingService>,
        entity_store: &Arc<std::sync::Mutex<EntityStore>>,
    ) -> Result<Vec<(String, String, Chunk, f32)>, String> {
        let query_vec = embedding_service
            .embed_text(query)
            .map_err(|e| e.to_string())?;

        let engine_guard = engine.blocking_lock();

        // Optional filter narrowing (D-04), mirroring search_documents_impl's
        // candidate-set combine truth table.
        let mut candidate_set: Option<std::collections::HashSet<String>> = None;
        if let Some(filters) = filters {
            let metadata_candidates =
                crate::search::filters::apply_metadata_filters(filters, &engine_guard)
                    .map_err(|e| e.to_string())?;
            let entity_candidates = {
                let entity_guard = entity_store
                    .lock()
                    .map_err(|e| format!("entity_store lock poisoned: {}", e))?;
                crate::search::filters::apply_entity_class_filters(
                    filters.entity_filters.as_deref().unwrap_or(&[]),
                    &entity_guard,
                )
            };
            candidate_set = match (metadata_candidates, entity_candidates) {
                (None, None) => None,
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
            };
        }

        // Query-mention entity narrowing: if the query text names a known
        // canonical entity (or alias), narrow candidates to docs mentioning
        // that entity. Only applied when the mention-set is small (<=3
        // entities) — larger mention sets usually mean the query is broad.
        let mention_ids = Self::entities_mentioned_in_query(query, entity_store);
        if !mention_ids.is_empty() && mention_ids.len() <= 3 {
            let entity_guard = entity_store
                .lock()
                .map_err(|e| format!("entity_store lock poisoned: {}", e))?;
            let mut mention_docs: std::collections::HashSet<String> = std::collections::HashSet::new();
            for id in &mention_ids {
                if let Some(docs) = entity_guard.doc_index.get(id) {
                    for d in docs { mention_docs.insert(d.clone()); }
                }
            }
            if !mention_docs.is_empty() {
                candidate_set = Some(match candidate_set {
                    None => mention_docs,
                    Some(existing) => existing.intersection(&mention_docs).cloned().collect(),
                });
            }
        }

        let collection_arc = engine_guard
            .collections
            .get_collection("documents_384")
            .ok_or_else(|| "documents_384 collection not found".to_string())?;

        // Extract years from the query. If any present, we bias retrieval:
        //   - HNSW k is enlarged so pre-filtered set is deep enough.
        //   - Docs whose path/excerpt mentions the year get a score boost.
        //   - Docs with a strong year-mismatch (a different explicit year in
        //     filename) are filtered out entirely when candidate_set is unset.
        let query_years = Self::extract_years(query);

        let k_hnsw = if query_years.is_empty() { 12 } else { 30 };

        let search_query = ruvector_core::types::SearchQuery {
            vector: query_vec.clone(),
            k: k_hnsw,
            filter: None,
            ef_search: None,
        };

        let raw_results = {
            let collection = collection_arc.read();
            collection.db.search(search_query).map_err(|e| e.to_string())?
        };

        let mut all_candidates: Vec<(String, String, Chunk, f32)> = Vec::new();

        for raw in raw_results {
            if let Some(ref candidates) = candidate_set {
                if !candidates.contains(&raw.id) {
                    continue;
                }
            }

            let metadata = match raw.metadata {
                Some(ref m) => m,
                None => continue,
            };

            let doc_title = metadata
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let doc_path = metadata
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Year-aware bias (generic — works for any query with a year token):
            //   * boost = +0.15 if title/path contains a query year
            //   * hard filter: drop the doc if title mentions a DIFFERENT explicit
            //     year and no query year, AND no other candidate matches; this is
            //     handled implicitly by the post-loop rerank.
            let mut year_boost: f32 = 0.0;
            if !query_years.is_empty() {
                let hay = format!("{} {}", doc_title, doc_path);
                let doc_years: Vec<String> = Self::find_years(&hay);
                let matches_query_year = query_years.iter().any(|qy| doc_years.contains(qy));
                if matches_query_year {
                    year_boost = 0.15;
                } else if !doc_years.is_empty() {
                    // Doc has a specific year AND no query-year match → skip.
                    // Only skip when candidate_set is None (no user-supplied filters).
                    if candidate_set.is_none() {
                        continue;
                    }
                }
            }

            // Read FULL document text from disk (Fix: 200-char excerpts starve LLM).
            // Falls back to metadata.excerpt if disk parse fails.
            let full_text: String = if !doc_path.is_empty() {
                crate::pipeline::parser::parse_document(std::path::Path::new(&doc_path))
                    .ok()
                    .map(|pd| pd.text)
                    .or_else(|| {
                        metadata
                            .get("excerpt")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default()
            } else {
                metadata
                    .get("excerpt")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            };

            if full_text.is_empty() {
                continue;
            }

            let doc_chunks = chunk_text(&full_text, 1500, 200);
            let mut scored_chunks: Vec<(String, String, Chunk, f32)> = Vec::new();

            for chunk in doc_chunks {
                let chunk_vec = match embedding_service.embed_text(&chunk.text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let base = cosine_sim(&query_vec, &chunk_vec);
                let score = (base + year_boost).clamp(-1.0, 1.0);
                scored_chunks.push((raw.id.clone(), doc_title.clone(), chunk, score));
            }

            scored_chunks.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
            scored_chunks.truncate(5);
            all_candidates.extend(scored_chunks);
        }

        all_candidates.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        all_candidates.truncate(15);
        Ok(all_candidates)
    }

    /// Persist the assistant's `ChatMessage` (with optional citations) to the
    /// session store and flush to disk. Failures are logged, not propagated —
    /// the streaming events have already reached the frontend by this point.
    async fn persist_assistant_message(
        chat_store: &Arc<Mutex<ChatSessionStore>>,
        app_data_dir: &PathBuf,
        session_id: &str,
        assistant_message_id: &str,
        content: &str,
        citations: Option<Vec<Citation>>,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let msg = ChatMessage {
            id: assistant_message_id.to_string(),
            role: ChatRole::Assistant,
            content: content.to_string(),
            citations,
            created_at: now.clone(),
        };

        let mut store = chat_store.lock().await;
        store.append_message(session_id, msg, now);
        if let Err(e) = store.save(app_data_dir) {
            eprintln!("Warning: failed to persist chat session {}: {}", session_id, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test 1: chunk_text 500/50 overlap ──
    #[test]
    fn test_chunk_text_500_char_50_overlap() {
        let text: String = "a".repeat(1200);
        let chunks = chunk_text(&text, 500, 50);
        assert_eq!(chunks.len(), 3, "1200-char input must produce 3 chunks");

        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, 500);
        assert_eq!(chunks[1].start, 450);
        assert_eq!(chunks[1].end, 950);
        assert_eq!(chunks[2].start, 900);
        assert_eq!(chunks[2].end, 1200);
    }

    // ── Test 2: short input → single chunk ──
    #[test]
    fn test_chunk_text_short_input_single_chunk() {
        let text = "short text under 500 chars";
        let chunks = chunk_text(text, 500, 50);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, text.chars().count() as u32);
    }

    // ── Test 3: cosine_sim ──
    #[test]
    fn test_cosine_similarity() {
        let identical = vec![1.0, 2.0, 3.0];
        assert!((cosine_sim(&identical, &identical) - 1.0).abs() < 1e-6);

        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_sim(&a, &b) - 0.0).abs() < 1e-6);

        // Known small pair: [1,0,0] vs [1,1,0] -> cos = 1/sqrt(2)
        let x = vec![1.0, 0.0, 0.0];
        let y = vec![1.0, 1.0, 0.0];
        let expected = 1.0 / std::f32::consts::SQRT_2;
        assert!((cosine_sim(&x, &y) - expected).abs() < 1e-5);
    }

    // ── Test 4: rerank_chunks top 12 of 24 ──
    #[test]
    fn test_rerank_top_12() {
        let mut candidates = Vec::new();
        for i in 0..24 {
            candidates.push((
                format!("doc-{}", i),
                Chunk {
                    text: format!("chunk-{}", i),
                    start: 0,
                    end: 10,
                },
                i as f32,
            ));
        }

        let top = rerank_chunks(candidates, 12);
        assert_eq!(top.len(), 12);
        // Descending order, highest score (23.0) first.
        for i in 0..12 {
            assert_eq!(top[i].2, (23 - i) as f32);
        }
    }

    // ── Test 5: build_rag_prompt exact shape ──
    #[test]
    fn test_build_rag_prompt() {
        let docs = vec![(1u32, "Doc A title", "chunk text 1"), (2u32, "Doc B title", "chunk text 2")];
        let prompt = build_rag_prompt(RAG_SYSTEM_PROMPT, &docs, "What happened?");
        let expected = "Documents in your library:\n[1] Doc A title: chunk text 1\n[2] Doc B title: chunk text 2\n\nQuestion: What happened?";
        assert_eq!(prompt, expected);
    }

    // ── Test 6: below-floor canned answer ──
    #[test]
    fn test_below_cosine_floor_returns_canned_answer() {
        assert_eq!(
            answer_or_canned(0.05),
            Some("I couldn't find anything relevant in your library.")
        );
        assert_eq!(answer_or_canned(0.19), Some(NO_MATCH_ANSWER));
        assert_eq!(answer_or_canned(0.20), None);
        assert_eq!(answer_or_canned(0.9), None);
    }

    // ── generate_suggestions (v1.2 #1: Chat suggestions) ──

    fn make_citation(index: u32, doc_title: &str) -> Citation {
        Citation {
            index,
            doc_id: format!("doc-{}", index),
            doc_title: doc_title.to_string(),
            chunk_start: 0,
            chunk_end: 10,
        }
    }

    #[test]
    fn test_suggestions_empty_when_no_citations() {
        let s = ChatEngine::generate_suggestions("how much total?", &[]);
        assert!(s.is_empty());
    }

    #[test]
    fn test_suggestions_aggregate_multi_year_offers_breakdown() {
        let citations = vec![
            make_citation(1, "Receipt_2020.pdf"),
            make_citation(2, "Receipt_2021.pdf"),
        ];
        let s = ChatEngine::generate_suggestions("how much total did I pay?", &citations);
        assert!(s.iter().any(|x| x.contains("break") || x.contains("year")));
    }

    #[test]
    fn test_suggestions_multi_doc_offers_source_list() {
        let citations = vec![
            make_citation(1, "Invoice A.pdf"),
            make_citation(2, "Invoice B.pdf"),
        ];
        let s = ChatEngine::generate_suggestions("what did I spend?", &citations);
        assert!(s.iter().any(|x| x.to_lowercase().contains("document")));
    }

    #[test]
    fn test_suggestions_capped_at_three() {
        let citations = vec![
            make_citation(1, "Receipt_2019.pdf"),
            make_citation(2, "Receipt_2020.pdf"),
            make_citation(3, "Receipt_2021.pdf"),
        ];
        let s = ChatEngine::generate_suggestions("what is the total amount?", &citations);
        assert!(s.len() <= 3);
    }

    #[test]
    fn test_suggestions_single_doc_non_aggregate_falls_back() {
        let citations = vec![make_citation(1, "Passport.pdf")];
        let s = ChatEngine::generate_suggestions("what is my passport number?", &citations);
        // No multi-year, no multi-doc, no aggregate → fallback template applies.
        assert_eq!(s, vec!["Can you show me more detail on this?".to_string()]);
    }

    // ── find_years: regression tests for the underscore-boundary bug ──
    // Bug: `\b(19|20)\d{2}\b` never matches "2020" in "Receipt_2020.pdf"
    // because `_` is a \w char, so there's no \b between "_" and "2".

    #[test]
    fn test_find_years_underscore_delimited_filename() {
        assert_eq!(
            ChatEngine::find_years("Prop_Riverside_P705_FY2016-17.pdf"),
            vec!["2016".to_string()]
        );
        assert_eq!(
            ChatEngine::find_years("Receipt_2020.pdf"),
            vec!["2020".to_string()]
        );
    }

    #[test]
    fn test_find_years_hyphen_delimited() {
        assert_eq!(
            ChatEngine::find_years("Invoice-2021-final.pdf"),
            vec!["2021".to_string()]
        );
    }

    #[test]
    fn test_find_years_plain_text() {
        assert_eq!(
            ChatEngine::find_years("how much property tax did I pay in 2025?"),
            vec!["2025".to_string()]
        );
    }

    #[test]
    fn test_find_years_ignores_non_year_digit_runs() {
        // 8-digit reference number must NOT be misread as two years or one year.
        assert!(ChatEngine::find_years("Ref 12345678 amount due").is_empty());
        // 5-digit run starting with 20 must NOT match (exact 4-digit only).
        assert!(ChatEngine::find_years("Order 20255 confirmed").is_empty());
    }

    #[test]
    fn test_find_years_multiple_distinct_years() {
        let years = ChatEngine::find_years("Compare_2019_vs_2022_report.pdf");
        assert_eq!(years, vec!["2019".to_string(), "2022".to_string()]);
    }

    #[test]
    fn test_find_years_dedups() {
        let years = ChatEngine::find_years("2020 tax year 2020 filing");
        assert_eq!(years, vec!["2020".to_string()]);
    }

    #[test]
    fn test_find_years_no_years_present() {
        assert!(ChatEngine::find_years("no dates here at all").is_empty());
    }

    // ── build_system_prompt (v1.2 #2: onboarding "About You" → RAG context) ──

    #[test]
    fn test_build_system_prompt_empty_profile_returns_base_unchanged() {
        let profile = crate::types::UserProfile::default();
        assert_eq!(build_system_prompt(&profile), RAG_SYSTEM_PROMPT);
    }

    #[test]
    fn test_build_system_prompt_name_only() {
        let profile = crate::types::UserProfile {
            display_name: "Alex Doe".to_string(),
            ..Default::default()
        };
        let prompt = build_system_prompt(&profile);
        assert!(prompt.starts_with(RAG_SYSTEM_PROMPT));
        assert!(prompt.contains("The user's name is Alex Doe."));
    }

    #[test]
    fn test_build_system_prompt_family_members() {
        let profile = crate::types::UserProfile {
            family_members: vec![
                crate::types::FamilyMember { name: "Jane Doe".to_string(), relation: "spouse".to_string() },
                crate::types::FamilyMember { name: "Sam Doe".to_string(), relation: "child".to_string() },
            ],
            ..Default::default()
        };
        let prompt = build_system_prompt(&profile);
        assert!(prompt.contains("Jane Doe (spouse)"));
        assert!(prompt.contains("Sam Doe (child)"));
    }

    #[test]
    fn test_build_system_prompt_full_profile() {
        let profile = crate::types::UserProfile {
            display_name: "Alex Doe".to_string(),
            aliases: vec!["A. Doe".to_string()],
            family_members: vec![crate::types::FamilyMember {
                name: "Jane Doe".to_string(),
                relation: "spouse".to_string(),
            }],
            countries: vec!["India".to_string(), "USA".to_string()],
            currencies: vec!["INR".to_string(), "USD".to_string()],
        };
        let prompt = build_system_prompt(&profile);
        assert!(prompt.contains("Alex Doe"));
        assert!(prompt.contains("A. Doe"));
        assert!(prompt.contains("Jane Doe (spouse)"));
        assert!(prompt.contains("India, USA"));
        assert!(prompt.contains("INR, USD"));
    }

    #[test]
    fn test_build_system_prompt_whitespace_only_name_treated_as_empty() {
        let profile = crate::types::UserProfile {
            display_name: "   ".to_string(),
            ..Default::default()
        };
        assert_eq!(build_system_prompt(&profile), RAG_SYSTEM_PROMPT);
    }
}
