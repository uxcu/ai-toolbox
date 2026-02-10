//! Codex Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use crate::coding::codex::apply_config_internal;
use crate::coding::db_id::db_clean_id;
use crate::db::DbState;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

/// Item for provider selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayProviderItem {
    pub id: String,
    pub display_name: String,
    pub is_selected: bool,
    pub is_disabled: bool,
    pub sort_index: i64,
}

/// Data for provider submenu
#[derive(Debug, Clone)]
pub struct TrayProviderData {
    pub title: String,
    pub items: Vec<TrayProviderItem>,
}

/// Get tray provider data for Codex
pub async fn get_codex_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM codex_provider")
        .await
        .map_err(|e| format!("Failed to query providers: {}", e))?
        .take(0);

    let mut items: Vec<TrayProviderItem> = Vec::new();

    if let Ok(records) = records_result {
        for record in records {
            if let (Some(raw_id), Some(name), Some(is_applied), sort_index) = (
                record.get("id").and_then(|v| v.as_str()),
                record.get("name").and_then(|v| v.as_str()),
                record.get("is_applied").and_then(|v| v.as_bool()),
                record
                    .get("sort_index")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            ) {
                let id = db_clean_id(raw_id);
                let is_disabled = record
                    .get("is_disabled")
                    .or_else(|| record.get("isDisabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                items.push(TrayProviderItem {
                    id,
                    display_name: name.to_string(),
                    is_selected: is_applied,
                    is_disabled,
                    sort_index,
                });
            }
        }
    }

    items.sort_by_key(|c| c.sort_index);

    Ok(TrayProviderData {
        title: "──── Codex ────".to_string(),
        items,
    })
}

/// Apply provider selection from tray menu
pub async fn apply_codex_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    apply_config_internal(&db, app, provider_id, true).await
}

/// Check if Codex should be shown in tray menu
pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}
