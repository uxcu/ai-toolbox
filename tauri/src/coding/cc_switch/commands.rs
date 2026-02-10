//! Tauri commands for CC-Switch import flow.

use chrono::{Local, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::State;

use super::adapter;
use super::types::{
    CcSwitchExport, CcSwitchImportResult, CcSwitchInstallationInfo, CcSwitchMcpServer,
    CcSwitchPreview, CcSwitchProvider,
    CcSwitchProviderStats, ImportOptions,
};
use crate::coding::claude_code::{adapter as claude_adapter, types::ClaudeCodeProviderContent};
use crate::coding::codex::{adapter as codex_adapter, types::CodexProviderContent};
use crate::coding::mcp::{mcp_store, types::McpServer};
use crate::db::DbState;

#[tauri::command]
pub async fn import_cc_switch_config(
    state: State<'_, DbState>,
    file_path: String,
    options: ImportOptions,
) -> Result<CcSwitchImportResult, String> {
    let file_content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read CC-Switch file '{}': {}", file_path, e))?;

    let export: CcSwitchExport = serde_json::from_str(&file_content)
        .map_err(|e| format!("Failed to parse CC-Switch JSON: {}", e))?;

    check_version_compatibility(&export.version)?;

    let mut provider_stats = CcSwitchProviderStats::default();
    let mut providers_skipped = 0;
    let mut mcp_count = 0;
    let mut mcp_servers_skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    if options.import_claude {
        for provider in &export.providers.claude {
            match import_claude_provider(&state, provider, options.skip_duplicates).await {
                Ok(imported) => {
                    if imported {
                        provider_stats.claude += 1;
                    } else {
                        providers_skipped += 1;
                    }
                }
                Err(e) => errors.push(format!("Claude provider '{}': {}", provider.name, e)),
            }
        }
    }

    if options.import_codex {
        for provider in &export.providers.codex {
            match import_codex_provider(&state, provider, options.skip_duplicates).await {
                Ok(imported) => {
                    if imported {
                        provider_stats.codex += 1;
                    } else {
                        providers_skipped += 1;
                    }
                }
                Err(e) => errors.push(format!("Codex provider '{}': {}", provider.name, e)),
            }
        }
    }

    if options.import_mcp {
        let (imported, skipped) = import_mcp_servers(
            &state,
            &export.mcp_servers,
            options.skip_duplicates,
            &mut errors,
        )
        .await;
        mcp_count = imported;
        mcp_servers_skipped = skipped;
    }

    let imported_total =
        provider_stats.claude + provider_stats.codex + provider_stats.gemini + mcp_count;
    let success = errors.is_empty() || imported_total > 0;

    Ok(CcSwitchImportResult {
        success,
        message: format!(
            "Imported {} Claude, {} Codex providers and {} MCP servers (skipped {} providers, {} MCP servers)",
            provider_stats.claude, provider_stats.codex, mcp_count, providers_skipped, mcp_servers_skipped
        ),
        provider_stats,
        mcp_count,
        providers_skipped,
        mcp_servers_skipped,
        errors,
    })
}

#[tauri::command]
pub async fn preview_cc_switch_config(file_path: String) -> Result<CcSwitchPreview, String> {
    let file_content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read CC-Switch file '{}': {}", file_path, e))?;

    let export: CcSwitchExport = serde_json::from_str(&file_content)
        .map_err(|e| format!("Failed to parse CC-Switch JSON: {}", e))?;

    check_version_compatibility(&export.version)?;

    let claude_count = export.providers.claude.len() as i32;
    let codex_count = export.providers.codex.len() as i32;
    let gemini_count = export.providers.gemini.len() as i32;
    let mcp_count = export.mcp_servers.len() as i32;

    let provider_stats = CcSwitchProviderStats {
        claude: claude_count,
        codex: codex_count,
        gemini: gemini_count,
    };

    serde_json::from_value(json!({
        "version": export.version,
        "export_time": export.export_time,
        "exportTime": export.export_time,
        "provider_stats": provider_stats,
        "providerStats": provider_stats,
        "mcp_count": mcp_count,
        "mcpCount": mcp_count,
        "provider_count": claude_count + codex_count + gemini_count,
        "providerCount": claude_count + codex_count + gemini_count,
    }))
    .map_err(|e| format!("Failed to build CC-Switch preview: {}", e))
}

#[tauri::command]
pub async fn detect_cc_switch_installation() -> Result<CcSwitchInstallationInfo, String> {
    let Some(config_dir) = get_cc_switch_config_dir() else {
        return Ok(CcSwitchInstallationInfo {
            installed: false,
            config_dir: None,
            version: None,
            last_export: None,
        });
    };

    let is_installed = match tokio::fs::metadata(&config_dir).await {
        Ok(metadata) => metadata.is_dir(),
        Err(_) => false,
    };

    if !is_installed {
        return Ok(CcSwitchInstallationInfo {
            installed: false,
            config_dir: None,
            version: None,
            last_export: None,
        });
    }

    Ok(CcSwitchInstallationInfo {
        installed: true,
        config_dir: Some(config_dir.to_string_lossy().to_string()),
        version: read_cc_switch_version(&config_dir).await,
        last_export: find_latest_backup_file(&config_dir).await,
    })
}

fn get_cc_switch_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        dirs::config_dir()
            .or_else(dirs::data_dir)
            .map(|dir| dir.join("cc-switch"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        dirs::home_dir().map(|dir| dir.join(".cc-switch"))
    }
}

async fn read_cc_switch_version(config_dir: &Path) -> Option<String> {
    let settings_path = config_dir.join("settings.json");
    let content = tokio::fs::read_to_string(&settings_path).await.ok()?;
    let payload: Value = serde_json::from_str(&content).ok()?;

    for key in ["version", "appVersion", "app_version"] {
        if let Some(value) = payload.get(key).and_then(|v| v.as_str()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    payload
        .get("app")
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

async fn find_latest_backup_file(config_dir: &Path) -> Option<String> {
    let backups_dir = config_dir.join("backups");
    let mut entries = tokio::fs::read_dir(&backups_dir).await.ok()?;
    let mut latest_backup: Option<(SystemTime, PathBuf)> = None;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let metadata = match entry.metadata().await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };

        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(_) => continue,
        };

        let should_update = latest_backup
            .as_ref()
            .map(|(latest_modified, _)| modified > *latest_modified)
            .unwrap_or(true);

        if should_update {
            latest_backup = Some((modified, entry.path()));
        }
    }

    latest_backup.map(|(_, path)| path.to_string_lossy().to_string())
}

pub fn check_version_compatibility(version: &str) -> Result<(), String> {
    if version.starts_with("3.") {
        return Ok(());
    }

    Err(format!(
        "Unsupported CC-Switch version '{}'. Only v3.x is supported.",
        version
    ))
}

async fn import_claude_provider(
    state: &State<'_, DbState>,
    cc_provider: &CcSwitchProvider,
    skip_duplicates: bool,
) -> Result<bool, String> {
    let provider_input = adapter::convert_to_claude_provider(cc_provider)?;

    if skip_duplicates
        && check_duplicate_provider(state, "claude_provider", &provider_input.name)
            .await?
            .is_some()
    {
        return Ok(false);
    }

    let now = Local::now().to_rfc3339();
    let content = ClaudeCodeProviderContent {
        name: provider_input.name,
        category: provider_input.category,
        settings_config: provider_input.settings_config,
        source_provider_id: provider_input.source_provider_id,
        website_url: provider_input.website_url,
        notes: provider_input.notes,
        icon: provider_input.icon,
        icon_color: provider_input.icon_color,
        sort_index: provider_input.sort_index,
        is_applied: false,
        is_disabled: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let db_payload = claude_adapter::to_db_value_provider(&content);
    let db = state.0.lock().await;

    db.query("CREATE claude_provider CONTENT $data")
        .bind(("data", db_payload))
        .await
        .map_err(|e| {
            format!(
                "Failed to create Claude provider '{}': {}",
                cc_provider.name, e
            )
        })?;

    Ok(true)
}

async fn import_codex_provider(
    state: &State<'_, DbState>,
    cc_provider: &CcSwitchProvider,
    skip_duplicates: bool,
) -> Result<bool, String> {
    let provider_input = adapter::convert_to_codex_provider(cc_provider)?;

    if skip_duplicates
        && check_duplicate_provider(state, "codex_provider", &provider_input.name)
            .await?
            .is_some()
    {
        return Ok(false);
    }

    let now = Local::now().to_rfc3339();
    let content = CodexProviderContent {
        name: provider_input.name,
        category: provider_input.category,
        settings_config: provider_input.settings_config,
        source_provider_id: provider_input.source_provider_id,
        website_url: provider_input.website_url,
        notes: provider_input.notes,
        icon: provider_input.icon,
        icon_color: provider_input.icon_color,
        sort_index: provider_input.sort_index,
        is_applied: false,
        is_disabled: provider_input.is_disabled.unwrap_or(false),
        created_at: now.clone(),
        updated_at: now,
    };

    let db_payload = codex_adapter::to_db_value_provider(&content);
    let db = state.0.lock().await;

    db.query("CREATE codex_provider CONTENT $data")
        .bind(("data", db_payload))
        .await
        .map_err(|e| {
            format!(
                "Failed to create Codex provider '{}': {}",
                cc_provider.name, e
            )
        })?;

    Ok(true)
}

async fn import_mcp_servers(
    state: &State<'_, DbState>,
    mcp_servers: &HashMap<String, CcSwitchMcpServer>,
    skip_duplicates: bool,
    errors: &mut Vec<String>,
) -> (i32, i32) {
    let mut imported = 0;
    let mut skipped = 0;

    for server in mcp_servers.values() {
        let converted = match adapter::convert_to_mcp_server(server) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("MCP server '{}': {}", server.name, e));
                continue;
            }
        };

        if skip_duplicates {
            match check_duplicate_mcp_server(state, &converted.name).await {
                Ok(Some(_)) => {
                    skipped += 1;
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    errors.push(format!(
                        "MCP server '{}': duplicate check failed: {}",
                        server.name, e
                    ));
                    continue;
                }
            }
        }

        let now = Utc::now().timestamp_millis();
        let mcp_server = McpServer {
            id: String::new(),
            name: converted.name,
            server_type: converted.server_type,
            server_config: converted.server_config,
            enabled_tools: converted.enabled_tools,
            sync_details: None,
            description: converted.description,
            tags: converted.tags,
            sort_index: 0,
            created_at: now,
            updated_at: now,
        };

        match mcp_store::upsert_mcp_server(state.inner(), &mcp_server).await {
            Ok(_) => imported += 1,
            Err(e) => errors.push(format!("MCP server '{}': {}", server.name, e)),
        }
    }

    (imported, skipped)
}

async fn check_duplicate_provider(
    state: &State<'_, DbState>,
    table: &str,
    provider_name: &str,
) -> Result<Option<Value>, String> {
    let sql = format!(
        "SELECT *, type::string(id) as id FROM {} WHERE name = $name LIMIT 1",
        table
    );
    let db = state.0.lock().await;

    let mut result = db
        .query(sql)
        .bind(("name", provider_name.trim().to_string()))
        .await
        .map_err(|e| {
            format!(
                "Failed to check existing provider '{}': {}",
                provider_name, e
            )
        })?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().next())
}

async fn check_duplicate_mcp_server(
    state: &State<'_, DbState>,
    server_name: &str,
) -> Result<Option<McpServer>, String> {
    mcp_store::get_mcp_server_by_name(state.inner(), server_name.trim())
        .await
        .map_err(|e| {
            format!(
                "Failed to check existing MCP server '{}': {}",
                server_name, e
            )
        })
}
