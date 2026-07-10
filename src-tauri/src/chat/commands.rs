//! IPC commands for Chat with Your Docs (Phase 11.7, Plan 05).
//!
//! Four `#[tauri::command] async fn`s:
//! - `start_chat`           — kicks off the RAG pipeline on a background task
//! - `list_chat_sessions`   — return all sessions, newest-first by updated_at
//! - `delete_chat_session`  — remove a session and persist
//! - `rename_chat_session`  — rename a session and persist
//!
//! `start_chat` persists the user message and generates the assistant
//! message id BEFORE spawning the background task, so the frontend can
//! attach its Tauri event listener using known ids before the first token
//! arrives. Errors from the spawned task are surfaced via `chat-stream-error`
//! events, never via `start_chat`'s own return value.
//!
//! Threat model (T-11.7-12): query length is capped at 4000 chars to bound
//! the size of the downstream embedding call and prompt.

use tauri::State;

use crate::auth::AuthState;
use crate::chat::engine::ChatEngine;
use crate::state::AppState;
use crate::types::{ChatSession, StartChatArgs};

/// Maximum accepted query length (T-11.7-12 DoS mitigation).
const MAX_QUERY_LEN: usize = 4000;

/// Response shape for `start_chat` — the ids the frontend needs to attach its
/// event listener before the first `chat-stream-token` arrives.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartChatResult {
    pub session_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
}

/// Derive a session title from the first ~40 chars of `query`, trimmed at a
/// word boundary (D-13).
fn derive_title(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.chars().count() <= 40 {
        return trimmed.to_string();
    }

    let truncated: String = trimmed.chars().take(40).collect();
    match truncated.rfind(' ') {
        Some(idx) if idx > 0 => truncated[..idx].to_string(),
        _ => truncated,
    }
}

/// Start (or continue) a RAG chat turn.
///
/// 1. Rejects empty or over-length queries.
/// 2. Resolves/creates the session.
/// 3. Persists the user's message.
/// 4. Spawns `ChatEngine::answer` on a background Tokio task (not awaited).
/// 5. Returns immediately with the ids the frontend needs to listen for
///    streaming events.
#[tauri::command]
pub async fn start_chat(
    app: tauri::AppHandle,
    args: StartChatArgs,
    state: State<'_, AppState>,
    auth: State<'_, AuthState>,
) -> Result<StartChatResult, String> {
    let trimmed_query = args.query.trim();
    if trimmed_query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    if trimmed_query.len() > MAX_QUERY_LEN {
        return Err(format!(
            "query must not exceed {} characters",
            MAX_QUERY_LEN
        ));
    }

    let now = chrono::Utc::now().to_rfc3339();

    let session_id = {
        let mut store = state.chat_session_store.lock().await;

        let resolved_id = match args.session_id {
            Some(id) => {
                if store.get(&id).is_none() {
                    return Err(format!("chat session not found: {}", id));
                }
                id
            }
            None => {
                let id = format!("cs-{}", uuid::Uuid::new_v4());
                let title = derive_title(trimmed_query);
                store.create_session(id.clone(), title, now.clone());
                id
            }
        };

        let user_message_id = format!("mid-{}", uuid::Uuid::new_v4());
        let user_msg = crate::types::ChatMessage {
            id: user_message_id,
            role: crate::types::ChatRole::User,
            content: trimmed_query.to_string(),
            citations: None,
            created_at: now.clone(),
        };
        store.append_message(&resolved_id, user_msg, now.clone());
        store
            .save(&state.app_data_dir)
            .map_err(|e| e.to_string())?;

        resolved_id
    };

    // Re-read the just-appended user message id (we need it distinctly from
    // the closure above for the response; regenerate deterministically is
    // wrong, so fetch it back from the store).
    let user_message_id = {
        let store = state.chat_session_store.lock().await;
        store
            .get(&session_id)
            .and_then(|s| s.messages.last())
            .map(|m| m.id.clone())
            .ok_or_else(|| "internal error: user message not found after append".to_string())?
    };

    let assistant_message_id = format!("mid-{}", uuid::Uuid::new_v4());

    // Clone the Arc handles needed by ChatEngine::answer.
    let auth_arc = std::sync::Arc::new(auth.inner().clone());
    let engine_arc = state.engine.clone();
    let embedding_service_arc = state.embedding_service.clone();
    let entity_store_arc = state.entity_store.clone();
    let chat_store_arc = state.chat_session_store.clone();
    let user_profile_arc = state.user_profile.clone();
    let app_data_dir = state.app_data_dir.clone();
    let query_owned = trimmed_query.to_string();
    let filters = args.filters;
    let session_id_for_task = session_id.clone();
    let user_message_id_for_task = user_message_id.clone();
    let assistant_message_id_for_task = assistant_message_id.clone();
    let app_for_task = app.clone();

    tokio::spawn(async move {
        if let Err(e) = ChatEngine::answer(
            app_for_task,
            auth_arc,
            engine_arc,
            embedding_service_arc,
            entity_store_arc,
            chat_store_arc,
            user_profile_arc,
            app_data_dir,
            session_id_for_task,
            user_message_id_for_task,
            assistant_message_id_for_task,
            query_owned,
            filters,
        )
        .await
        {
            eprintln!("ChatEngine::answer failed: {}", e);
        }
    });

    Ok(StartChatResult {
        session_id,
        user_message_id,
        assistant_message_id,
    })
}

/// Return all chat sessions, newest-first by `updated_at`.
#[tauri::command]
pub async fn list_chat_sessions(state: State<'_, AppState>) -> Result<Vec<ChatSession>, String> {
    let store = state.chat_session_store.lock().await;
    let mut sessions = store.all().to_vec();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// Remove a chat session and persist.
#[tauri::command]
pub async fn delete_chat_session(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.chat_session_store.lock().await;
    if !store.remove(&id) {
        return Err(format!("chat session not found: {}", id));
    }
    store
        .save(&state.app_data_dir)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Rename a chat session and persist.
#[tauri::command]
pub async fn rename_chat_session(
    id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.chat_session_store.lock().await;
    let found = store.rename_session(&id, title)?;
    if !found {
        return Err(format!("chat session not found: {}", id));
    }
    store
        .save(&state.app_data_dir)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_title_short_query_unchanged() {
        assert_eq!(derive_title("When did I buy Unit 204?"), "When did I buy Unit 204?");
    }

    #[test]
    fn test_derive_title_long_query_trimmed_at_word_boundary() {
        let long_query = "What were the total charges across all of my property tax invoices from last year and the year before that";
        let title = derive_title(long_query);
        assert!(title.chars().count() <= 40, "title must not exceed 40 chars, got {}", title.chars().count());
        assert!(!title.is_empty());
        assert!(long_query.starts_with(&title));
    }
}
