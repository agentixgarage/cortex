//! Persistent store for the user profile (v1.2 #2: onboarding "About You").
//!
//! Writes `{app_data_dir}/user_profile.json` — mirrors the JSON-sidecar
//! pattern used by `chat/session_store.rs` and `saved_searches/store.rs`.
//!
//! `load` never panics — any I/O or JSON parse error silently returns
//! `UserProfile::default()` (all-empty profile), matching the resilience
//! contract of the other sidecar stores in this codebase.

use crate::types::UserProfile;
use std::path::Path;

/// Load the user profile from `{app_data_dir}/user_profile.json`.
/// Returns `UserProfile::default()` on any I/O or parse error, or if the
/// file has never been written (first run / user skipped onboarding).
pub fn load(app_data_dir: &Path) -> UserProfile {
    let path = app_data_dir.join("user_profile.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the user profile to `{app_data_dir}/user_profile.json`.
/// Creates the file if absent; overwrites if present.
pub fn save(profile: &UserProfile, app_data_dir: &Path) -> std::io::Result<()> {
    let path = app_data_dir.join("user_profile.json");
    let json = serde_json::to_string_pretty(profile)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FamilyMember;

    #[test]
    fn test_load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let profile = load(dir.path());
        assert_eq!(profile, UserProfile::default());
    }

    #[test]
    fn test_load_corrupt_json_returns_default_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("user_profile.json"), "{not valid json").unwrap();
        let profile = load(dir.path());
        assert_eq!(profile, UserProfile::default());
    }

    #[test]
    fn test_save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let profile = UserProfile {
            display_name: "Alex Doe".to_string(),
            aliases: vec!["A. Doe".to_string(), "Alexander Doe".to_string()],
            family_members: vec![
                FamilyMember { name: "Jane Doe".to_string(), relation: "spouse".to_string() },
                FamilyMember { name: "Sam Doe".to_string(), relation: "child".to_string() },
            ],
            countries: vec!["India".to_string(), "United States".to_string()],
            currencies: vec!["INR".to_string(), "USD".to_string()],
        };
        save(&profile, dir.path()).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded, profile);
    }

    #[test]
    fn test_save_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let first = UserProfile {
            display_name: "First Name".to_string(),
            ..Default::default()
        };
        save(&first, dir.path()).unwrap();

        let second = UserProfile {
            display_name: "Second Name".to_string(),
            ..Default::default()
        };
        save(&second, dir.path()).unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded.display_name, "Second Name");
    }

    #[test]
    fn test_empty_profile_serializes_with_empty_collections() {
        let dir = tempfile::tempdir().unwrap();
        let profile = UserProfile::default();
        save(&profile, dir.path()).unwrap();
        let raw = std::fs::read_to_string(dir.path().join("user_profile.json")).unwrap();
        assert!(raw.contains("\"aliases\": []"));
        assert!(raw.contains("\"familyMembers\": []"));
    }
}
