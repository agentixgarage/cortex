//! IPC commands for the daily quiz feedback loop (v1.2 #3, ROADMAP.md).

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;
use crate::types::{QuizAnswer, QuizFeedbackEntry, QuizQuestion};

/// Return up to `limit` quiz candidates (default 1 — the Dashboard daily
/// card; the "Take more quizzes" tab under Insights passes a higher limit,
/// e.g. 10). Already-answered questions (by id) are excluded so the same
/// pair is never asked twice.
#[tauri::command]
pub async fn get_daily_quiz(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<QuizQuestion>, AppError> {
    let limit = limit.unwrap_or(1);
    let entity_store = state.entity_store.clone();
    let feedback = state.quiz_feedback.clone();

    let answered = {
        let guard = feedback.lock().await;
        guard.answered_question_ids()
    };

    let questions = tokio::task::spawn_blocking(move || {
        let store = entity_store
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // Over-generate then filter+truncate, since some candidates may
        // already be answered.
        let candidates = crate::quiz::generate_alias_candidates(&store.canonicals, limit * 5 + 10);
        Ok::<Vec<QuizQuestion>, AppError>(candidates)
    })
    .await??;

    let filtered: Vec<QuizQuestion> = questions
        .into_iter()
        .filter(|q| !answered.contains(&q.id))
        .take(limit)
        .collect();

    Ok(filtered)
}

/// Log the user's answer to a quiz question. See `quiz/mod.rs` module doc
/// for the explicit scope note: this records feedback but does not (yet)
/// execute an entity merge on "confirmed: true".
#[tauri::command]
pub async fn submit_quiz_answer(
    answer: QuizAnswer,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let entry = QuizFeedbackEntry {
        question_id: answer.question_id,
        kind: answer.kind,
        entity_id_a: answer.entity_id_a,
        entity_id_b: answer.entity_id_b,
        confirmed: answer.confirmed,
        answered_at: chrono::Utc::now().to_rfc3339(),
    };

    let app_data_dir = state.app_data_dir.clone();
    let mut store = state.quiz_feedback.lock().await;
    store.append(entry);
    store
        .save(&app_data_dir)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}
