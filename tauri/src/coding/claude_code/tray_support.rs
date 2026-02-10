//! Claude Code Tray Support Module
//!
//! Provides standardized API for tray menu integration.

use crate::coding::claude_code::apply_config_internal;
use crate::coding::db_id::db_clean_id;
use crate::db::DbState;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

/// Item for provider selection in tray menu
#[derive(Debug, Clone)]
pub struct TrayProviderItem {
    /// Provider ID (used in event handling)
    pub id: String,
    /// Display name in menu
    pub display_name: String,
    /// Whether this provider is currently selected/applied
    pub is_selected: bool,
    /// Whether this provider is disabled
    pub is_disabled: bool,
    /// Sort index for ordering
    pub sort_index: i64,
}

/// Data for provider submenu
#[derive(Debug, Clone)]
pub struct TrayProviderData {
    /// Title of the section
    pub title: String,
    /// Items for selection
    pub items: Vec<TrayProviderItem>,
}

/// Get tray provider data for Claude Code
pub async fn get_claude_code_tray_data<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TrayProviderData, String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    // Query providers from database
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM claude_provider")
        .await
        .map_err(|e| format!("Failed to query providers: {}", e))?
        .take(0);

    let mut items: Vec<TrayProviderItem> = Vec::new();

    match records_result {
        Ok(records) => {
            for record in records {
                if let (Some(raw_id), Some(name), Some(is_applied), sort_index) = (
                    record.get("id").and_then(|v| v.as_str()),
                    record.get("name").and_then(|v| v.as_str()),
                    record
                        .get("is_applied")
                        .or_else(|| record.get("isApplied"))
                        .and_then(|v| v.as_bool()),
                    record
                        .get("sort_index")
                        .or_else(|| record.get("sortIndex"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0),
                ) {
                    let id = db_clean_id(raw_id);

                    // Read is_disabled field
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
        Err(e) => {
            eprintln!("Failed to deserialize providers for tray: {}", e);
        }
    }

    // Sort by sort_index
    items.sort_by_key(|c| c.sort_index);

    let data = TrayProviderData {
        title: "──── Claude Code ────".to_string(),
        items: items
            .into_iter()
            .map(|mut item| {
                item.sort_index = 0; // Clear sort_index for tray display
                item
            })
            .collect(),
    };

    Ok(data)
}

/// Apply provider selection from tray menu
pub async fn apply_claude_code_provider<R: Runtime>(
    app: &AppHandle<R>,
    provider_id: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();
    let db = state.0.lock().await;

    apply_config_internal(&db, app, provider_id, true).await?;

    Ok(())
}

/// Check if Claude Code should be shown in tray menu
/// Returns true - Claude Code is always visible as a core feature
pub async fn is_enabled_for_tray<R: Runtime>(_app: &AppHandle<R>) -> bool {
    true
}
