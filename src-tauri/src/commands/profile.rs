//! IPC commands for the user profile (v1.2 #2: onboarding "About You" step).

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;
use crate::types::UserProfile;

/// Return the current user profile. All-empty defaults if the user has
/// never filled it in (onboarding skipped, or pre-v1.2 install).
#[tauri::command]
pub async fn get_user_profile(state: State<'_, AppState>) -> Result<UserProfile, AppError> {
    let profile = state.user_profile.lock().await;
    Ok(profile.clone())
}

/// Replace the user profile wholesale and persist to disk.
/// Called from onboarding's "About You" step and from Settings if the user
/// edits their profile later.
#[tauri::command]
pub async fn save_user_profile(
    profile: UserProfile,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let app_data_dir = state.app_data_dir.clone();
    crate::profile::store::save(&profile, &app_data_dir)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut guard = state.user_profile.lock().await;
    *guard = profile;
    Ok(())
}
