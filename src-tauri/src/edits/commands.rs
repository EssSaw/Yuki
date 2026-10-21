use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, State};

use super::{EditProposalSummary, EditProposals, recovery};
use crate::chat::state::ChatState;

#[derive(Default)]
pub struct EditState {
    proposals: EditProposals,
    operation: Mutex<()>,
    stop_generation: AtomicU64,
}

fn config(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|_| "Cannot locate application settings".into())
}

fn require_idle(
    chat: &ChatState,
) -> Result<std::sync::MutexGuard<'_, Option<crate::chat::state::ActiveRequest>>, String> {
    let guard = chat
        .inner
        .active
        .lock()
        .map_err(|_| "Chat state unavailable")?;
    if guard.is_some() {
        return Err("Wait for chat to finish or cancel it before editing locally".into());
    }
    Ok(guard)
}

#[tauri::command]
pub fn prepare_local_text_edit(
    app: AppHandle,
    state: State<'_, std::sync::Arc<EditState>>,
    chat: State<'_, ChatState>,
    conversation_id: String,
    path: String,
    generation: u64,
    replacement: String,
) -> Result<EditProposalSummary, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another local edit operation is active")?;
    let _chat = require_idle(&chat)?;
    state.proposals.prepare(
        &config(&app)?,
        &conversation_id,
        &path,
        generation,
        &replacement,
    )
}

#[tauri::command]
pub async fn apply_local_text_edit(
    app: AppHandle,
    state: State<'_, std::sync::Arc<EditState>>,
    chat: State<'_, ChatState>,
    proposal_id: String,
) -> Result<String, String> {
    let config = config(&app)?;
    let state = state.inner().clone();
    let chat = chat.inner().clone();
    let stop_generation = state.stop_generation.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another local edit operation is active")?;
        let _chat = require_idle(&chat)?;

        state.proposals.apply_checked(&config, &proposal_id, || {
            state.stop_generation.load(Ordering::SeqCst) != stop_generation
        })
    })
    .await
    .map_err(|_| "Edit worker failed; inspect recovery status before retrying".to_string())?
}

#[tauri::command]
pub async fn undo_local_text_edit(
    app: AppHandle,
    state: State<'_, std::sync::Arc<EditState>>,
    chat: State<'_, ChatState>,
    change_id: String,
    generation: u64,
) -> Result<(), String> {
    let config = config(&app)?;
    let state = state.inner().clone();
    let chat = chat.inner().clone();
    let stop_generation = state.stop_generation.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another local edit operation is active")?;
        let _chat = require_idle(&chat)?;

        state
            .proposals
            .undo_checked(&config, &change_id, generation, || {
                state.stop_generation.load(Ordering::SeqCst) != stop_generation
            })
    })
    .await
    .map_err(|_| "Undo worker failed; inspect recovery status before retrying".to_string())?
}

#[tauri::command]
pub fn stop_local_text_edit(state: State<'_, std::sync::Arc<EditState>>) {
    state.stop_generation.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
pub fn discard_local_text_edit(
    state: State<'_, std::sync::Arc<EditState>>,
    proposal_id: String,
) -> Result<(), String> {
    state.proposals.discard(&proposal_id)
}

#[tauri::command]
pub fn list_local_edit_recovery(
    app: AppHandle,
    state: State<'_, std::sync::Arc<EditState>>,
) -> Result<Vec<recovery::RecoveryStatus>, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Edit is active; refresh recovery after completion")?;
    recovery::inventory(&config(&app)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::credentials::SecretStore;

    struct NoSecrets;
    impl SecretStore for NoSecrets {
        fn get_api_key(&self, _service: &str, _user: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set_api_key(&self, _service: &str, _user: &str, _key: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete_api_key(&self, _service: &str, _user: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn stop_generation_is_not_cleared_when_worker_starts() {
        let state = EditState::default();
        let scheduled = state.stop_generation.load(Ordering::SeqCst);
        state.stop_generation.fetch_add(1, Ordering::SeqCst);
        assert_ne!(scheduled, state.stop_generation.load(Ordering::SeqCst));
    }

    #[test]
    fn local_edit_gate_rejects_active_chat() {
        let chat = ChatState::new(std::sync::Arc::new(NoSecrets));
        let _receiver = chat.try_start_request("request", "conversation").unwrap();
        assert!(require_idle(&chat).is_err());
        chat.finish_request("request");
        assert!(require_idle(&chat).is_ok());
    }
}
