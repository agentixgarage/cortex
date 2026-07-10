//! Persistent store for quiz feedback (v1.2 #3, ROADMAP.md).
//!
//! Writes `{app_data_dir}/quiz_feedback.json` — mirrors the JSON-sidecar
//! pattern used by `chat/session_store.rs` and `profile/store.rs`.
//!
//! `load` never panics — any I/O or JSON parse error silently returns an
//! empty store, matching the resilience contract of the other sidecar
//! stores in this codebase.

use crate::types::QuizFeedbackEntry;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuizFeedbackStore {
    pub entries: Vec<QuizFeedbackEntry>,
}

impl QuizFeedbackStore {
    pub fn load(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("quiz_feedback.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app_data_dir: &Path) -> std::io::Result<()> {
        let path = app_data_dir.join("quiz_feedback.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    pub fn append(&mut self, entry: QuizFeedbackEntry) {
        self.entries.push(entry);
    }

    /// Ids of entities already asked about (either side of the pair) —
    /// used to avoid re-asking the same alias_confirm question after the
    /// user has answered it once, even across daily-card refreshes.
    pub fn answered_question_ids(&self) -> std::collections::HashSet<String> {
        self.entries.iter().map(|e| e.question_id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::QuizFeedbackEntry;

    fn make_entry(question_id: &str) -> QuizFeedbackEntry {
        QuizFeedbackEntry {
            question_id: question_id.to_string(),
            kind: "alias_confirm".to_string(),
            entity_id_a: "a".to_string(),
            entity_id_b: "b".to_string(),
            confirmed: true,
            answered_at: "2026-07-10T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = QuizFeedbackStore::load(dir.path());
        assert!(store.entries.is_empty());
    }

    #[test]
    fn test_load_corrupt_json_returns_empty_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("quiz_feedback.json"), "{not valid").unwrap();
        let store = QuizFeedbackStore::load(dir.path());
        assert!(store.entries.is_empty());
    }

    #[test]
    fn test_save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = QuizFeedbackStore::default();
        store.append(make_entry("q-1"));
        store.append(make_entry("q-2"));
        store.save(dir.path()).unwrap();

        let loaded = QuizFeedbackStore::load(dir.path());
        assert_eq!(loaded.entries.len(), 2);
    }

    #[test]
    fn test_answered_question_ids() {
        let mut store = QuizFeedbackStore::default();
        store.append(make_entry("q-1"));
        store.append(make_entry("q-2"));
        let ids = store.answered_question_ids();
        assert!(ids.contains("q-1"));
        assert!(ids.contains("q-2"));
        assert_eq!(ids.len(), 2);
    }
}
