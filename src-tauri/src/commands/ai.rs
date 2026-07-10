use tauri::{AppHandle, Emitter, State};
use serde::Serialize;
use crate::auth::AuthState;
use crate::auth::commands::{ProviderAuthStatus, LoginRequest};
use crate::auth::oauth::{
    map_oauth_error, OAuthStartResult,
    validate_openai_token, validate_gemini_token, validate_ollama_endpoint,
};
use crate::ai::service::{AIServiceRequest, AIServiceResponse, ServiceMessage};

/// Kick off the OpenAI Codex OAuth PKCE flow.
///
/// Opens the system browser to Codex's authorization URL. When the user
/// completes the flow, the loopback listener captures the callback, Cortex
/// exchanges the code for access + refresh tokens, and stores them under
/// provider slug "openai-codex" as an OAuth credential.
///
/// Frontend invocation:
///   tauriInvoke("start_openai_oauth") → ProviderAuthStatus (provider="openai-codex")
#[tauri::command]
pub async fn start_openai_oauth(
    app: tauri::AppHandle,
    auth: State<'_, AuthState>,
) -> Result<ProviderAuthStatus, String> {
    crate::auth::oauth::start_openai_codex_oauth(&app, &auth).await?;

    // Return refreshed status for "openai-codex" — reuse existing list_providers path
    let statuses = crate::auth::commands::get_auth_status(auth)?;
    statuses
        .into_iter()
        .find(|s| s.provider == "openai-codex")
        .ok_or_else(|| "openai-codex credential missing after OAuth flow".to_string())
}

/// List authentication status for all supported AI providers.
///
/// Returns a flat list with one entry per provider.
/// Frontend invocation: `tauriInvoke("list_providers")` → `ProviderAuthStatus[]`
#[tauri::command]
pub fn list_providers(auth: State<'_, AuthState>) -> Result<Vec<ProviderAuthStatus>, String> {
    crate::auth::commands::get_auth_status(auth)
}

/// Connect (authenticate) a provider with API key or Ollama config.
///
/// D-08 VALIDATION-BEFORE-STORE: performs a real HTTP validation call BEFORE
/// delegating to login_provider. If validation fails, the credential is never
/// stored — the store is only written on validation success.
///
/// Supported methods and providers:
/// - method="api-key", provider="openai" → validate_openai_token
/// - method="api-key", provider="gemini" → validate_gemini_token
/// - method="ollama" (any provider) → validate_ollama_endpoint
/// - method="api-key" for other providers → Err (Anthropic uses save_setup_token)
///
/// Frontend invocation: `tauriInvoke("connect_provider", { request: {...} })` → `ProviderAuthStatus`
#[tauri::command]
pub async fn connect_provider(
    auth: State<'_, AuthState>,
    request: LoginRequest,
) -> Result<ProviderAuthStatus, String> {
    match request.method.as_str() {
        "api-key" => {
            let credential = request.credential.as_deref().ok_or("API key is required")?;
            match request.provider.as_str() {
                "openai" => {
                    validate_openai_token(credential)
                        .await
                        .map_err(|e| map_oauth_error(&e))?;
                }
                "gemini" => {
                    validate_gemini_token(credential)
                        .await
                        .map_err(|e| map_oauth_error(&e))?;
                }
                p => {
                    return Err(format!(
                        "API-key auth not supported for provider: {}. \
                         Anthropic uses save_setup_token.",
                        p
                    ));
                }
            }
        }
        "ollama" => {
            let base_url = request
                .base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            validate_ollama_endpoint(base_url)
                .await
                .map_err(|e| map_oauth_error(&e))?;
        }
        other => {
            return Err(format!("Unsupported auth method: {}", other));
        }
    }

    // Validation passed — now delegate to auth/commands.rs to persist the credential.
    // login_provider is sync; calling it here after the async validation is fine.
    crate::auth::commands::login_provider(auth, request)
}

/// Disconnect (remove) credentials for a provider.
///
/// D-24 §6: best-effort revoke POST before local credential removal.
/// Revoke failure does NOT block disconnect — local deletion ALWAYS proceeds.
///
/// Frontend invocation: `tauriInvoke("disconnect_provider", { provider: "openai" })`
#[tauri::command]
pub async fn disconnect_provider(
    auth: State<'_, AuthState>,
    provider: String,
) -> Result<(), String> {
    crate::auth::commands::logout_provider(auth, provider).await
}

/// Set the active AI provider for chat and entity extraction.
///
/// Frontend invocation: `tauriInvoke("set_active_provider", { provider: "anthropic" })`
#[tauri::command]
pub fn set_active_provider(
    auth: State<'_, AuthState>,
    provider: String,
) -> Result<(), String> {
    crate::auth::commands::set_active_provider(auth, provider)
}

/// Change the model on an already-connected provider without disconnecting.
///
/// Fixes a real gap: previously the only way to change a provider's model
/// was full disconnect+reconnect, and OAuth providers (openai-codex) have
/// no model picker in their connect flow at all — the model was
/// permanently stuck at whatever default was chosen on first connect.
///
/// Frontend invocation: `tauriInvoke("set_provider_model", { provider: "openai-codex", model: "gpt-4o" })`
#[tauri::command]
pub fn set_provider_model(
    auth: State<'_, AuthState>,
    provider: String,
    model: String,
) -> Result<(), String> {
    auth.update_credential_model(&provider, &model)
}

/// Get the currently active AI provider key.
///
/// Frontend invocation: `tauriInvoke("get_active_provider")` → `string | null`
#[tauri::command]
pub fn get_active_provider(auth: State<'_, AuthState>) -> Result<Option<String>, String> {
    auth.get_active_provider()
}

/// Save an Anthropic Claude setup token (sk-ant-oat01-*).
///
/// Validates the token against the Anthropic API (D-08 — validate_anthropic_token
/// is called inside save_setup_token in auth/oauth.rs) before storing it.
///
/// Frontend invocation: `tauriInvoke("save_setup_token", { token: "sk-ant-oat01-..." })` → `OAuthStartResult`
#[tauri::command]
pub async fn save_setup_token(
    auth: State<'_, AuthState>,
    token: String,
) -> Result<OAuthStartResult, String> {
    crate::auth::oauth::save_setup_token(auth, token).await
}

/// Test the connection to the currently active AI provider.
///
/// Sends a minimal 1-token request to the active provider's chat endpoint.
/// Useful for "Ollama connectivity check" (roadmap criterion 3) and post-connect
/// diagnostics. Returns Ok(()) on success, Err with mapped error on failure.
///
/// Frontend invocation: `tauriInvoke("test_connection", { provider: "ollama" })`
#[tauri::command]
pub async fn test_connection(
    auth: State<'_, AuthState>,
    _provider: String,
) -> Result<(), String> {
    let request = AIServiceRequest {
        system_prompt: String::new(),
        messages: vec![ServiceMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        max_tokens: Some(1),
        temperature: None,
        response_format: None,
        model_override: None,
    };
    crate::ai::service::ai_request(auth.inner(), request)
        .await
        .map(|_| ())
        .map_err(|e| map_oauth_error(&e))
}

/// Send a chat message to the active AI provider with exponential-backoff retry.
///
/// Wraps ai_request_with_retry with max_retries=2 (RESEARCH Open Question #3 decision).
/// Initial delay is 2000ms, doubling on each retry (max total wait: 6s for 2 retries).
///
/// Frontend invocation: `tauriInvoke("chat", { request: {...AIServiceRequest} })` → `AIServiceResponse`
#[tauri::command]
pub async fn chat(
    auth: State<'_, AuthState>,
    request: AIServiceRequest,
) -> Result<AIServiceResponse, String> {
    crate::ai::retry::ai_request_with_retry(auth.inner(), request, 2).await
}

/// Progress payload for the `ruvllm://download-progress` Tauri event (11.8-06 Task 3).
///
/// DEVIATION from the plan's literal byte-level progress spec: ruvllm's `load_model()`
/// downloads via `hf_hub::api::sync::Api` internally, which does not expose a byte-level
/// progress callback. This event therefore emits coarse-grained steps (0/1 at start, 1/1
/// on completion) rather than a continuous byte counter. Documented in 11.8-06-SUMMARY.md.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuvllmDownloadProgress {
    pub model_id: String,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub status: String, // "downloading" | "complete" | "error"
    pub error: Option<String>,
}

/// Download (and load) a local-ruvllm model by HF repo id, emitting
/// `ruvllm://download-progress` events as it proceeds.
///
/// D-04: model selection dialog surfaces this as "Download model" — the allowlist check
/// inside `ai::ruvllm::download_model` prevents an arbitrary/unvalidated HF Hub download
/// (Rule 2 — missing-input-validation is a correctness requirement, not a nice-to-have).
///
/// Frontend invocation: `tauriInvoke("download_ruvllm_model", { modelId: "Qwen/..." })` → `String` (local path)
#[tauri::command]
pub async fn download_ruvllm_model(
    model_id: String,
    app: AppHandle,
    auth: State<'_, AuthState>,
) -> Result<String, String> {
    let app_data_dir = auth
        .store_path
        .parent()
        .ok_or_else(|| "could not resolve app data dir".to_string())?
        .to_path_buf();

    let app_for_progress = app.clone();
    let model_id_for_progress = model_id.clone();

    let result = crate::ai::ruvllm::download_model(&model_id, &app_data_dir, move |downloaded, total| {
        let _ = app_for_progress.emit(
            "ruvllm://download-progress",
            RuvllmDownloadProgress {
                model_id: model_id_for_progress.clone(),
                bytes_downloaded: downloaded,
                total_bytes: total,
                status: "downloading".to_string(),
                error: None,
            },
        );
    })
    .await;

    match result {
        Ok(path) => {
            let _ = app.emit(
                "ruvllm://download-progress",
                RuvllmDownloadProgress {
                    model_id: model_id.clone(),
                    bytes_downloaded: 1,
                    total_bytes: 1,
                    status: "complete".to_string(),
                    error: None,
                },
            );
            Ok(path.to_string_lossy().to_string())
        }
        Err(e) => {
            let _ = app.emit(
                "ruvllm://download-progress",
                RuvllmDownloadProgress {
                    model_id: model_id.clone(),
                    bytes_downloaded: 0,
                    total_bytes: 1,
                    status: "error".to_string(),
                    error: Some(e.clone()),
                },
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;

    /// Test 2: Command validates model_id against an allowlist — unknown ids return
    /// an error mentioning "unknown model id". Verified directly against the underlying
    /// ai::ruvllm::download_model allowlist check (the Tauri command wraps it 1:1 with
    /// no additional validation, so this exercises the same code path the command uses).
    #[tokio::test]
    async fn test_download_model_rejects_unknown_model_id() {
        let dir = tempfile::tempdir().unwrap();
        let result = crate::ai::ruvllm::download_model(
            "totally/unknown-model-id",
            dir.path(),
            |_, _| {},
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown model id"));
    }

    /// Test 4 (partial — full Tauri AppHandle path requires a running app; this test
    /// verifies the underlying storage-path contract that download_ruvllm_model relies on
    /// for the "atomic rename lands the file at model_storage_path" requirement is at least
    /// well-defined and deterministic given app_data_dir + model_id).
    #[test]
    fn test_model_storage_path_matches_download_target() {
        let dir = tempfile::tempdir().unwrap();
        let model_id = "Qwen/Qwen2.5-7B-Instruct-GGUF";
        let expected = crate::ai::ruvllm::model_storage_path(dir.path(), model_id);
        assert_eq!(
            expected,
            dir.path().join("ruvllm-models").join(model_id)
        );
    }
}
