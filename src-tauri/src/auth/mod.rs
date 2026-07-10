pub mod commands;
pub mod loopback;
pub mod oauth;
pub mod pkce;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Known AI providers eligible for auto-promotion to `active_provider`.
/// `"anthropic"` is the legacy canonical id for Claude; `"claude"` is the id
/// used by the current Settings UI provider list. Non-AI credentials sharing
/// this store (e.g. `"youtube"`) are deliberately excluded (CR-02).
const AI_PROVIDERS: [&str; 6] = ["claude", "anthropic", "openai", "openai-codex", "gemini", "ollama"];

/// Returns true if `provider` is a known AI provider that may become the
/// active provider when none is set yet.
fn is_ai_provider(provider: &str) -> bool {
    AI_PROVIDERS.contains(&provider)
}

/// Supported auth methods for AI providers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    OAuth,
    ApiKey,
    None, // Ollama / local
}

/// Per-provider credential storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    pub provider: String,
    pub method: AuthMethod,
    pub api_key: Option<String>,
    pub oauth_token: Option<String>,
    pub display_name: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    // NEW — Phase 7 gap closure (D-22..D-25): OAuth refresh token and expiry.
    // Both fields use #[serde(default)] so existing credentials.json files
    // (written before this plan) parse without error — missing keys → None.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>, // Unix epoch seconds
}

/// Persistent credential store backed by a JSON file.
/// Credentials are stored in the Tauri app data directory.
#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct CredentialStore {
    pub(crate) active_provider: Option<String>,
    pub(crate) credentials: HashMap<String, ProviderCredential>,
}

#[derive(Clone)]
pub struct AuthState {
    pub(crate) store_path: PathBuf,
    pub(crate) store: Arc<Mutex<CredentialStore>>,
}

impl AuthState {
    pub fn new(state_dir: &PathBuf) -> Self {
        let store_path = state_dir.join("credentials.json");
        let store = if store_path.exists() {
            match std::fs::read_to_string(&store_path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
                Err(_) => CredentialStore::default(),
            }
        } else {
            CredentialStore::default()
        };

        Self {
            store_path,
            store: Arc::new(Mutex::new(store)),
        }
    }

    pub(crate) fn persist(&self) -> Result<(), String> {
        let store = self.store.lock().map_err(|e| e.to_string())?;
        let data = serde_json::to_string_pretty(&*store).map_err(|e| e.to_string())?;
        std::fs::write(&self.store_path, data).map_err(|e| e.to_string())
    }

    pub fn store_api_key(
        &self,
        provider: &str,
        api_key: &str,
        model: Option<&str>,
    ) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        store.credentials.insert(
            provider.to_string(),
            ProviderCredential {
                provider: provider.to_string(),
                method: AuthMethod::ApiKey,
                api_key: Some(api_key.to_string()),
                oauth_token: None,
                display_name: None,
                model: model.map(String::from),
                base_url: None,
                refresh_token: None,
                expires_at: None,
            },
        );
        // Only auto-promote KNOWN AI providers to active_provider. A non-AI
        // credential reusing this store (e.g. the YouTube Data API key) must
        // never silently hijack the active AI provider — doing so breaks
        // downstream ai_request resolution while the UI shows "Not connected"
        // (CR-02).
        if store.active_provider.is_none() && is_ai_provider(provider) {
            store.active_provider = Some(provider.to_string());
        }
        drop(store);
        self.persist()
    }

    pub fn store_ollama_config(
        &self,
        base_url: &str,
        model: Option<&str>,
    ) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        store.credentials.insert(
            "ollama".to_string(),
            ProviderCredential {
                provider: "ollama".to_string(),
                method: AuthMethod::None,
                api_key: None,
                oauth_token: None,
                display_name: Some("Ollama (Local)".to_string()),
                model: model.map(String::from),
                base_url: Some(base_url.to_string()),
                refresh_token: None,
                expires_at: None,
            },
        );
        drop(store);
        self.persist()
    }

    /// Store a setup-token (Claude) or OAuth token for a provider.
    /// The token is stored in oauth_token and method is set to OAuth.
    pub fn store_oauth_token(
        &self,
        provider: &str,
        token: &str,
        display_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        store.credentials.insert(
            provider.to_string(),
            ProviderCredential {
                provider: provider.to_string(),
                method: AuthMethod::OAuth,
                api_key: None,
                oauth_token: Some(token.to_string()),
                display_name: display_name.map(String::from),
                model: model.map(String::from),
                base_url: None,
                refresh_token: None,
                expires_at: None,
            },
        );
        if store.active_provider.is_none() {
            store.active_provider = Some(provider.to_string());
        }
        drop(store);
        self.persist()
    }

    pub fn get_credential(&self, provider: &str) -> Result<Option<ProviderCredential>, String> {
        let store = self.store.lock().map_err(|e| e.to_string())?;
        Ok(store.credentials.get(provider).cloned())
    }

    pub fn get_active_provider(&self) -> Result<Option<String>, String> {
        let store = self.store.lock().map_err(|e| e.to_string())?;
        Ok(store.active_provider.clone())
    }

    pub fn set_active_provider(&self, provider: &str) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        if !store.credentials.contains_key(provider) {
            return Err(format!("No credentials stored for provider: {}", provider));
        }
        store.active_provider = Some(provider.to_string());
        drop(store);
        self.persist()
    }

    /// Update the `model` field on an already-stored credential in place —
    /// no re-authentication needed. Fixes a real UX gap: previously the
    /// only way to change a provider's model was full disconnect+reconnect,
    /// and for OAuth providers (openai-codex) the reconnect flow itself has
    /// no model picker at all, so the model was permanently stuck at
    /// whatever default was set on first connect.
    pub fn update_credential_model(&self, provider: &str, model: &str) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        let cred = store
            .credentials
            .get_mut(provider)
            .ok_or_else(|| format!("No credentials stored for provider: {}", provider))?;
        cred.model = Some(model.to_string());
        drop(store);
        self.persist()
    }

    pub fn remove_credential(&self, provider: &str) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        store.credentials.remove(provider);
        if store.active_provider.as_deref() == Some(provider) {
            store.active_provider = store.credentials.keys().next().cloned();
        }
        drop(store);
        self.persist()
    }

    pub fn list_credentials(&self) -> Result<Vec<ProviderCredential>, String> {
        let store = self.store.lock().map_err(|e| e.to_string())?;
        Ok(store.credentials.values().cloned().collect())
    }

    /// Store an OAuth token with optional refresh_token and expires_at.
    /// Used by plan 07-09 `start_openai_codex_oauth` after code exchange.
    pub fn store_oauth_credential_with_refresh(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<i64>,
        display_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        store.credentials.insert(
            provider.to_string(),
            ProviderCredential {
                provider: provider.to_string(),
                method: AuthMethod::OAuth,
                api_key: None,
                oauth_token: Some(access_token.to_string()),
                display_name: display_name.map(String::from),
                model: model.map(String::from),
                base_url: None,
                refresh_token: refresh_token.map(String::from),
                expires_at,
            },
        );
        if store.active_provider.is_none() && is_ai_provider(provider) {
            store.active_provider = Some(provider.to_string());
        }
        drop(store);
        self.persist()
    }

    /// Update the access_token, refresh_token, and expires_at for a stored credential.
    /// Used by ai/service.rs after a successful token refresh. Persists to disk.
    ///
    /// If the refresh response omits refresh_token (allowed per Codex spec), the existing
    /// stored refresh_token is preserved. Same for expires_at.
    pub fn update_oauth_tokens(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        let existing = store
            .credentials
            .get(provider)
            .cloned()
            .ok_or_else(|| format!("No credential found for provider: {}", provider))?;

        store.credentials.insert(
            provider.to_string(),
            ProviderCredential {
                oauth_token: Some(access_token.to_string()),
                // Preserve existing refresh_token if provider didn't return a new one
                refresh_token: refresh_token
                    .map(String::from)
                    .or(existing.refresh_token.clone()),
                // Preserve existing expires_at if provider didn't return a new one
                expires_at: expires_at.or(existing.expires_at),
                ..existing
            },
        );
        drop(store);
        self.persist()
    }

    /// Get the active credential, resolving to the active provider.
    pub fn get_active_credential(&self) -> Result<Option<ProviderCredential>, String> {
        let store = self.store.lock().map_err(|e| e.to_string())?;
        let active = match &store.active_provider {
            Some(p) => p,
            None => return Ok(None),
        };
        Ok(store.credentials.get(active).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_auth_state() -> (AuthState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let state = AuthState::new(&dir.path().to_path_buf());
        (state, dir)
    }

    #[test]
    fn test_new_creates_empty_store() {
        let (state, _dir) = temp_auth_state();
        assert!(state.get_active_provider().unwrap().is_none());
        assert!(state.list_credentials().unwrap().is_empty());
    }

    #[test]
    fn test_store_api_key() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("anthropic", "sk-test-123", Some("claude-sonnet-4-20250514")).unwrap();

        let cred = state.get_credential("anthropic").unwrap().unwrap();
        assert_eq!(cred.provider, "anthropic");
        assert_eq!(cred.method, AuthMethod::ApiKey);
        assert_eq!(cred.api_key.as_deref(), Some("sk-test-123"));
        assert_eq!(cred.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn test_first_stored_becomes_active() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", None).unwrap();

        assert_eq!(state.get_active_provider().unwrap().as_deref(), Some("openai"));
    }

    #[test]
    fn test_youtube_key_does_not_become_active_provider() {
        // CR-02: storing a non-AI credential (youtube) must NOT auto-promote it
        // to active_provider, even when no AI provider is configured yet.
        let (state, _dir) = temp_auth_state();
        state.store_api_key("youtube", "AIza-test", None).unwrap();

        assert!(
            state.get_active_provider().unwrap().is_none(),
            "youtube must never become the active AI provider"
        );
        // The credential itself is still stored and retrievable.
        let cred = state.get_credential("youtube").unwrap().unwrap();
        assert_eq!(cred.api_key.as_deref(), Some("AIza-test"));
    }

    #[test]
    fn test_youtube_before_ai_provider_lets_ai_take_active() {
        // CR-02: even if youtube is stored first, the next AI provider stored
        // becomes active (because youtube never claimed the active slot).
        let (state, _dir) = temp_auth_state();
        state.store_api_key("youtube", "AIza-test", None).unwrap();
        state.store_api_key("openai", "sk-openai", None).unwrap();

        assert_eq!(
            state.get_active_provider().unwrap().as_deref(),
            Some("openai"),
            "first AI provider becomes active despite youtube being stored first"
        );
    }

    #[test]
    fn test_second_stored_does_not_change_active() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", None).unwrap();
        state.store_api_key("anthropic", "sk-ant", None).unwrap();

        assert_eq!(state.get_active_provider().unwrap().as_deref(), Some("openai"));
    }

    #[test]
    fn test_set_active_provider() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", None).unwrap();
        state.store_api_key("anthropic", "sk-ant", None).unwrap();
        state.set_active_provider("anthropic").unwrap();

        assert_eq!(state.get_active_provider().unwrap().as_deref(), Some("anthropic"));
    }

    #[test]
    fn test_set_active_provider_unknown_fails() {
        let (state, _dir) = temp_auth_state();
        let result = state.set_active_provider("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_update_credential_model_changes_in_place() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", Some("gpt-4o-mini")).unwrap();

        state.update_credential_model("openai", "gpt-4.1").unwrap();

        let cred = state.get_credential("openai").unwrap().unwrap();
        assert_eq!(cred.model.as_deref(), Some("gpt-4.1"));
    }

    #[test]
    fn test_update_credential_model_preserves_other_fields() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", Some("gpt-4o-mini")).unwrap();

        state.update_credential_model("openai", "gpt-4.1").unwrap();

        let cred = state.get_credential("openai").unwrap().unwrap();
        assert_eq!(cred.api_key.as_deref(), Some("sk-openai"));
        assert_eq!(cred.provider, "openai");
    }

    #[test]
    fn test_update_credential_model_oauth_provider() {
        let (state, _dir) = temp_auth_state();
        let now_plus_3600 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 3600)
            .unwrap_or(0);
        state
            .store_oauth_credential_with_refresh(
                "openai-codex",
                "at_xxx",
                Some("rt_yyy"),
                Some(now_plus_3600),
                Some("ChatGPT (Codex)"),
                Some("gpt-4o"),
            )
            .unwrap();

        state.update_credential_model("openai-codex", "o1-mini").unwrap();

        let cred = state.get_credential("openai-codex").unwrap().unwrap();
        assert_eq!(cred.model.as_deref(), Some("o1-mini"));
        // OAuth token must survive the model update untouched.
        assert_eq!(cred.oauth_token.as_deref(), Some("at_xxx"));
    }

    #[test]
    fn test_update_credential_model_unknown_provider_fails() {
        let (state, _dir) = temp_auth_state();
        let result = state.update_credential_model("nonexistent", "some-model");
        assert!(result.is_err());
    }

    #[test]
    fn test_store_ollama_config() {
        let (state, _dir) = temp_auth_state();
        state.store_ollama_config("http://localhost:11434", Some("llama3")).unwrap();

        let cred = state.get_credential("ollama").unwrap().unwrap();
        assert_eq!(cred.method, AuthMethod::None);
        assert_eq!(cred.base_url.as_deref(), Some("http://localhost:11434"));
        assert_eq!(cred.model.as_deref(), Some("llama3"));
        assert!(cred.api_key.is_none());
    }

    #[test]
    fn test_remove_credential() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", None).unwrap();
        state.remove_credential("openai").unwrap();

        assert!(state.get_credential("openai").unwrap().is_none());
        assert!(state.list_credentials().unwrap().is_empty());
    }

    #[test]
    fn test_remove_active_falls_back() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-openai", None).unwrap();
        state.store_api_key("anthropic", "sk-ant", None).unwrap();
        state.remove_credential("openai").unwrap();

        // Active should fall back to remaining provider
        let active = state.get_active_provider().unwrap();
        assert!(active.is_some());
        assert_eq!(active.as_deref(), Some("anthropic"));
    }

    #[test]
    fn test_get_active_credential() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("anthropic", "sk-ant-123", None).unwrap();

        let cred = state.get_active_credential().unwrap().unwrap();
        assert_eq!(cred.provider, "anthropic");
        assert_eq!(cred.api_key.as_deref(), Some("sk-ant-123"));
    }

    #[test]
    fn test_get_active_credential_none_when_empty() {
        let (state, _dir) = temp_auth_state();
        assert!(state.get_active_credential().unwrap().is_none());
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Store credential
        {
            let state = AuthState::new(&path);
            state.store_api_key("anthropic", "sk-persist", Some("claude-sonnet-4-20250514")).unwrap();
        }

        // Load from same path
        {
            let state = AuthState::new(&path);
            let cred = state.get_credential("anthropic").unwrap().unwrap();
            assert_eq!(cred.api_key.as_deref(), Some("sk-persist"));
            assert_eq!(state.get_active_provider().unwrap().as_deref(), Some("anthropic"));
        }
    }

    #[test]
    fn test_list_credentials() {
        let (state, _dir) = temp_auth_state();
        state.store_api_key("openai", "sk-1", None).unwrap();
        state.store_api_key("anthropic", "sk-2", None).unwrap();
        state.store_ollama_config("http://localhost:11434", None).unwrap();

        let creds = state.list_credentials().unwrap();
        assert_eq!(creds.len(), 3);
    }

    #[test]
    fn test_corrupt_file_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let cred_path = dir.path().join("credentials.json");
        fs::write(&cred_path, "not valid json").unwrap();

        let state = AuthState::new(&dir.path().to_path_buf());
        assert!(state.list_credentials().unwrap().is_empty());
    }

    // --- Plan 07-08: ProviderCredential backward-compat tests ---

    #[test]
    fn test_provider_credential_roundtrips_without_new_fields() {
        // A credential created BEFORE plan 07-08 (no refresh_token / expires_at)
        // must round-trip through serde_json with new fields as None.
        let cred = ProviderCredential {
            provider: "anthropic".to_string(),
            method: AuthMethod::OAuth,
            api_key: None,
            oauth_token: Some("sk-ant-oat01-test".to_string()),
            display_name: Some("Claude".to_string()),
            model: Some("claude-haiku-4-5".to_string()),
            base_url: None,
            refresh_token: None,
            expires_at: None,
        };
        let json = serde_json::to_string(&cred).unwrap();
        let restored: ProviderCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.provider, "anthropic");
        assert_eq!(restored.oauth_token.as_deref(), Some("sk-ant-oat01-test"));
        assert!(restored.refresh_token.is_none());
        assert!(restored.expires_at.is_none());
    }

    #[test]
    fn test_provider_credential_roundtrips_with_new_fields() {
        let cred = ProviderCredential {
            provider: "openai-codex".to_string(),
            method: AuthMethod::OAuth,
            api_key: None,
            oauth_token: Some("access_token_value".to_string()),
            display_name: None,
            model: None,
            base_url: None,
            refresh_token: Some("rt_xxx".to_string()),
            expires_at: Some(1_800_000_000i64),
        };
        let json = serde_json::to_string(&cred).unwrap();
        let restored: ProviderCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.refresh_token.as_deref(), Some("rt_xxx"));
        assert_eq!(restored.expires_at, Some(1_800_000_000i64));
    }

    #[test]
    fn test_legacy_credentials_json_parses() {
        // Simulate a v1.1-ship credentials.json that has NO refresh_token / expires_at keys.
        // AuthState::new must parse it without error and return the credential.
        let dir = tempfile::tempdir().unwrap();
        let cred_path = dir.path().join("credentials.json");
        // "method" must use the kebab-case serialized form that serde writes:
        // OAuth → "o-auth", ApiKey → "api-key", None → "none"
        let legacy_json = r#"{
            "active_provider": "anthropic",
            "credentials": {
                "anthropic": {
                    "provider": "anthropic",
                    "method": "o-auth",
                    "api_key": null,
                    "oauth_token": "sk-ant-oat01-legacy",
                    "display_name": "Claude",
                    "model": "claude-haiku-4-5",
                    "base_url": null
                }
            }
        }"#;
        fs::write(&cred_path, legacy_json).unwrap();

        let state = AuthState::new(&dir.path().to_path_buf());
        let cred = state.get_credential("anthropic").unwrap().unwrap();
        assert_eq!(cred.oauth_token.as_deref(), Some("sk-ant-oat01-legacy"));
        assert!(cred.refresh_token.is_none(), "legacy credential should have None refresh_token");
        assert!(cred.expires_at.is_none(), "legacy credential should have None expires_at");
    }

    #[test]
    fn test_credential_with_expires_at_zero_is_valid() {
        // expires_at: Some(0) is epoch 1970 — legally stored as "already expired".
        // Only None means "no expiry". Some(0) is NOT treated as None.
        let cred = ProviderCredential {
            provider: "openai-codex".to_string(),
            method: AuthMethod::OAuth,
            api_key: None,
            oauth_token: Some("tok".to_string()),
            display_name: None,
            model: None,
            base_url: None,
            refresh_token: None,
            expires_at: Some(0i64),
        };
        let json = serde_json::to_string(&cred).unwrap();
        let restored: ProviderCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.expires_at, Some(0i64),
            "expires_at: Some(0) must round-trip as Some(0), not become None");
    }

    // --- Plan 07-09: Task 1 tests for store_oauth_credential_with_refresh + AI_PROVIDERS ---

    #[test]
    fn test_store_oauth_credential_with_refresh_sets_all_fields() {
        let (state, _dir) = temp_auth_state();
        let now_plus_3600 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 3600)
            .unwrap_or(0);

        state.store_oauth_credential_with_refresh(
            "openai-codex",
            "at_xxx",
            Some("rt_yyy"),
            Some(now_plus_3600),
            Some("ChatGPT (Codex)"),
            Some("gpt-5"),
        ).unwrap();

        let cred = state.get_credential("openai-codex").unwrap().unwrap();
        assert_eq!(cred.provider, "openai-codex");
        assert_eq!(cred.method, AuthMethod::OAuth);
        assert_eq!(cred.oauth_token.as_deref(), Some("at_xxx"));
        assert_eq!(cred.refresh_token.as_deref(), Some("rt_yyy"));
        assert_eq!(cred.expires_at, Some(now_plus_3600));
        assert_eq!(cred.display_name.as_deref(), Some("ChatGPT (Codex)"));
        assert_eq!(cred.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn test_store_oauth_credential_with_refresh_auto_promotes_active() {
        let (state, _dir) = temp_auth_state();
        // Empty store — first AI provider stored should become active
        state.store_oauth_credential_with_refresh(
            "openai-codex",
            "at_xxx",
            Some("rt_yyy"),
            None,
            None,
            None,
        ).unwrap();

        assert_eq!(
            state.get_active_provider().unwrap().as_deref(),
            Some("openai-codex"),
            "openai-codex must auto-promote to active when no provider is set"
        );
    }

    #[test]
    fn test_store_oauth_credential_with_refresh_does_not_auto_promote_non_ai() {
        let (state, _dir) = temp_auth_state();
        // Storing a non-AI provider must NOT claim the active slot
        state.store_oauth_credential_with_refresh(
            "random-thing",
            "tok",
            None,
            None,
            None,
            None,
        ).unwrap();

        assert!(
            state.get_active_provider().unwrap().is_none(),
            "random-thing must NOT become the active AI provider"
        );
    }

    #[test]
    fn test_ai_providers_admits_openai_codex() {
        // openai-codex must be in the AI_PROVIDERS allow-list
        assert!(
            is_ai_provider("openai-codex"),
            "openai-codex must be recognized as an AI provider (CR-02 allow-list)"
        );
        // Sanity: a random string is NOT an AI provider
        assert!(!is_ai_provider("random-thing"));
    }

    #[test]
    fn test_store_oauth_credential_with_refresh_persists_after_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Store via first instance
        {
            let state = AuthState::new(&path);
            state.store_oauth_credential_with_refresh(
                "openai-codex",
                "at_persisted",
                Some("rt_persisted"),
                Some(9999999999i64),
                Some("ChatGPT (Codex)"),
                None,
            ).unwrap();
        }

        // Re-instantiate from same path — credential must survive
        {
            let state = AuthState::new(&path);
            let cred = state.get_credential("openai-codex").unwrap().unwrap();
            assert_eq!(cred.oauth_token.as_deref(), Some("at_persisted"));
            assert_eq!(cred.refresh_token.as_deref(), Some("rt_persisted"));
            assert_eq!(cred.expires_at, Some(9999999999i64));
        }
    }
}
