use super::{sync, adapter};
use super::types::{FileMapping, SyncProgress, SyncResult, WSLErrorResult, WSLDetectResult, WSLStatusResult, WSLSyncConfig};
use crate::db::DbState;
use crate::coding::{open_code, oh_my_opencode, oh_my_opencode_slim};
use tauri::Emitter;
use chrono::Local;

// ============================================================================
// WSL Detection Commands
// ============================================================================

/// Detect WSL availability and get distro list
#[tauri::command]
pub fn wsl_detect() -> WSLDetectResult {
    sync::detect_wsl()
}

/// Check if a specific WSL distro is available
#[tauri::command]
pub fn wsl_check_distro(distro: String) -> WSLErrorResult {
    match sync::get_wsl_distros() {
        Ok(distros) => {
            let available = distros.contains(&distro);
            WSLErrorResult {
                available,
                error: if available { None } else { Some(format!("Distro '{}' not found", distro)) },
            }
        }
        Err(e) => WSLErrorResult {
            available: false,
            error: Some(e),
        },
    }
}

/// Get running state of a specific WSL distro
#[tauri::command]
pub fn wsl_get_distro_state(distro: String) -> String {
    sync::get_wsl_distro_state(&distro)
}

// ============================================================================
// WSL Config Commands
// ============================================================================

/// Get WSL sync configuration
#[tauri::command]
pub async fn wsl_get_config(state: tauri::State<'_, DbState>) -> Result<WSLSyncConfig, String> {
    let db = state.0.lock().await;

    // Get config
    let config_result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM wsl_sync_config:`config` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query WSL config: {}", e))?
        .take(0);

    let config = match config_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                adapter::config_from_db_value(record.clone(), vec![])
            } else {
                WSLSyncConfig::default()
            }
        }
        Err(_) => WSLSyncConfig::default(),
    };

    // Get file mappings
    let mappings_result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM wsl_file_mapping ORDER BY module, name")
        .await
        .map_err(|e| format!("Failed to query file mappings: {}", e))?
        .take(0);

    let file_mappings = match mappings_result {
        Ok(records) => {
            records
                .into_iter()
                .map(adapter::mapping_from_db_value)
                .collect()
        }
        Err(_) => vec![],
    };

    Ok(WSLSyncConfig {
        file_mappings,
        ..config
    })
}

/// Save WSL sync configuration
#[tauri::command]
pub async fn wsl_save_config(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    config: WSLSyncConfig,
) -> Result<(), String> {
    // Check if WSL sync is being enabled (was disabled, now enabled)
    let was_enabled = {
        let db = state.0.lock().await;
        let result: Result<Vec<serde_json::Value>, _> = db
            .query("SELECT enabled FROM wsl_sync_config:`config` LIMIT 1")
            .await
            .map_err(|e| format!("Failed to query WSL config: {}", e))?
            .take(0);
        result
            .ok()
            .and_then(|records| records.first().cloned())
            .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
            .unwrap_or(false)
    };

    let is_being_enabled = !was_enabled && config.enabled;

    {
        let db = state.0.lock().await;

        // Save config
        let config_data = adapter::config_to_db_value(&config);
        db.query("UPSERT wsl_sync_config:`config` CONTENT $data")
            .bind(("data", config_data))
            .await
            .map_err(|e| format!("Failed to save WSL config: {}", e))?;

        // Update file mappings - follow open_code/free_models pattern: use backtick format table:`id`
        for mapping in config.file_mappings.iter() {
            let mapping_data = adapter::mapping_to_db_value(mapping);
            let query = format!("UPSERT wsl_file_mapping:`{}` CONTENT $data", mapping.id);
            db.query(&query)
                .bind(("data", mapping_data))
                .await
                .map_err(|e| format!("Failed to save file mapping: {}", e))?;
        }
    }

    // Emit event to refresh UI
    let _ = app.emit("wsl-config-changed", ());

    // If WSL sync was just enabled, trigger a full sync
    if is_being_enabled {
        log::info!("WSL sync enabled, triggering full sync...");

        let result = do_full_sync(&state, &app, &config, None).await;

        if !result.errors.is_empty() {
            log::warn!("WSL full sync errors: {:?}", result.errors);
        }

        // Update sync status
        update_sync_status(state.inner(), &result).await?;

        // Emit sync completed event
        let _ = app.emit("wsl-sync-completed", result);
    }

    Ok(())
}

// ============================================================================
// File Mapping Commands
// ============================================================================

/// Add a new file mapping
#[tauri::command]
pub async fn wsl_add_file_mapping(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    mapping: FileMapping,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let mapping_data = adapter::mapping_to_db_value(&mapping);
    db.query("UPSERT type::thing('wsl_file_mapping', $id) CONTENT $data")
        .bind(("id", mapping.id.clone()))
        .bind(("data", mapping_data))
        .await
        .map_err(|e| format!("Failed to add file mapping: {}", e))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

/// Update an existing file mapping
#[tauri::command]
pub async fn wsl_update_file_mapping(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    mapping: FileMapping,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let mapping_data = adapter::mapping_to_db_value(&mapping);
    db.query("UPSERT type::thing('wsl_file_mapping', $id) CONTENT $data")
        .bind(("id", mapping.id.clone()))
        .bind(("data", mapping_data))
        .await
        .map_err(|e| format!("Failed to update file mapping: {}", e))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

/// Delete a file mapping
#[tauri::command]
pub async fn wsl_delete_file_mapping(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE type::thing('wsl_file_mapping', $id)")
        .bind(("id", id))
        .await
        .map_err(|e| format!("Failed to delete file mapping: {}", e))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

/// Delete all file mappings (reset)
#[tauri::command]
pub async fn wsl_reset_file_mappings(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE wsl_file_mapping")
        .await
        .map_err(|e| format!("Failed to reset file mappings: {}", e))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

// ============================================================================
// Sync Commands
// ============================================================================

/// Internal full sync implementation (reusable)
pub(super) async fn do_full_sync(
    state: &DbState,
    app: &tauri::AppHandle,
    config: &WSLSyncConfig,
    module: Option<&str>,
) -> SyncResult {
    // Get effective distro (auto-resolve if configured one doesn't exist)
    let distro = match sync::get_effective_distro(&config.distro) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("WSL full sync skipped: {}", e);
            return SyncResult {
                success: false,
                synced_files: vec![],
                skipped_files: vec![],
                errors: vec![e],
            };
        }
    };

    // Emit initial progress for file mappings
    let enabled_mappings: Vec<_> = config.file_mappings.iter().filter(|m| m.enabled).collect();
    let total_files = enabled_mappings.len() as u32;
    let _ = app.emit("wsl-sync-progress", SyncProgress {
        phase: "files".to_string(),
        current_item: "准备中...".to_string(),
        current: 0,
        total: total_files,
        message: format!("文件同步: 0/{}", total_files),
    });

    // Dynamically resolve config file paths for opencode and oh-my-opencode
    let file_mappings = resolve_dynamic_paths(config.file_mappings.clone());

    // Sync file mappings with progress
    let mut result = sync_mappings_with_progress(&file_mappings, &distro, module, app);

    // Also sync MCP and Skills to WSL (full sync)
    if config.sync_mcp {
        if let Err(e) = super::mcp_sync::sync_mcp_to_wsl(state, app.clone()).await {
            log::warn!("MCP WSL sync failed: {}", e);
            result.errors.push(format!("MCP sync: {}", e));
            result.success = false;
        }
    }
    if config.sync_skills {
        if let Err(e) = super::skills_sync::sync_skills_to_wsl(state, app.clone()).await {
            log::warn!("Skills WSL sync failed: {}", e);
            result.errors.push(format!("Skills sync: {}", e));
            result.success = false;
        }
    }

    result
}

/// Sync file mappings with progress events
fn sync_mappings_with_progress(
    mappings: &[FileMapping],
    distro: &str,
    module_filter: Option<&str>,
    app: &tauri::AppHandle,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];

    let filtered_mappings: Vec<_> = mappings
        .iter()
        .filter(|m| m.enabled)
        .filter(|m| module_filter.is_none() || Some(m.module.as_str()) == module_filter)
        .collect();

    let total = filtered_mappings.len() as u32;

    for (idx, mapping) in filtered_mappings.iter().enumerate() {
        let current = (idx + 1) as u32;
        
        // Emit progress
        let _ = app.emit("wsl-sync-progress", SyncProgress {
            phase: "files".to_string(),
            current_item: mapping.name.clone(),
            current,
            total,
            message: format!("文件同步: {}/{} - {}", current, total, mapping.name),
        });

        match sync::sync_file_mapping(mapping, distro) {
            Ok(files) if files.is_empty() => {
                skipped_files.push(mapping.name.clone());
            }
            Ok(files) => {
                synced_files.extend(files);
            }
            Err(e) => {
                errors.push(format!("{}: {}", mapping.name, e));
            }
        }
    }

    SyncResult {
        success: errors.is_empty(),
        synced_files,
        skipped_files,
        errors,
    }
}

/// Sync all files or specific module to WSL
#[tauri::command]
pub async fn wsl_sync(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    module: Option<String>,
) -> Result<SyncResult, String> {
    let config = wsl_get_config(state.clone()).await?;

    if !config.enabled {
        return Ok(SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec!["WSL sync is not enabled".to_string()],
        });
    }

    let result = do_full_sync(&state, &app, &config, module.as_deref()).await;

    // Update sync status
    update_sync_status(state.inner(), &result).await?;

    // Emit event to update UI
    let _ = app.emit("wsl-sync-completed", result.clone());

    Ok(result)
}

/// Get current WSL sync status
#[tauri::command]
pub async fn wsl_get_status(state: tauri::State<'_, DbState>) -> Result<WSLStatusResult, String> {
    let config = wsl_get_config(state).await?;

    let wsl_available = if config.enabled {
        match sync::get_wsl_distros() {
            Ok(distros) => distros.contains(&config.distro),
            Err(_) => false,
        }
    } else {
        false
    };

    Ok(WSLStatusResult {
        wsl_available,
        last_sync_time: config.last_sync_time,
        last_sync_status: config.last_sync_status,
        last_sync_error: config.last_sync_error,
    })
}

/// Test if a Windows path exists and can be accessed
#[tauri::command]
pub fn wsl_test_path(windows_path: String) -> Result<bool, String> {
    let expanded = sync::expand_env_vars(&windows_path)?;
    Ok(std::path::Path::new(&expanded).exists())
}

/// Get default file mappings
#[tauri::command]
pub fn wsl_get_default_mappings() -> Vec<FileMapping> {
    default_file_mappings()
}

// ============================================================================
// WSL UI Commands
// ============================================================================

/// Open WSL terminal for a specific distro
#[tauri::command]
#[cfg(target_os = "windows")]
pub fn wsl_open_terminal(distro: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    std::process::Command::new("cmd")
        .args(["/c", "start", "wsl", "-d", &distro, "--cd", "~"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to open WSL terminal: {}", e))?;

    Ok(())
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
pub fn wsl_open_terminal(_distro: String) -> Result<(), String> {
    Err("WSL is only available on Windows".to_string())
}

/// Open Windows Explorer to WSL user's home directory
#[tauri::command]
#[cfg(target_os = "windows")]
pub fn wsl_open_folder(distro: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // Get actual home directory from WSL (handles root user whose home is /root, not /home/root)
    let output = std::process::Command::new("wsl")
        .args(["-d", &distro, "--exec", "bash", "-c", "echo $HOME"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to get WSL home directory: {}", e))?;

    let home_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if home_dir.is_empty() {
        return Err("Failed to get WSL home directory".to_string());
    }

    // Convert WSL path (e.g. /root or /home/user) to UNC path: \\wsl$\<distro>\root or \\wsl$\<distro>\home\user
    let home_unix = home_dir.replace('/', "\\");
    let wsl_path = format!(r"\\wsl$\{}{}", distro, home_unix);
    std::process::Command::new("explorer.exe")
        .arg(&wsl_path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to open WSL folder: {}", e))?;

    Ok(())
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
pub fn wsl_open_folder(_distro: String) -> Result<(), String> {
    Err("WSL is only available on Windows".to_string())
}

// ============================================================================
// Internal Functions
// ============================================================================

/// Dynamically resolve config file paths for opencode and oh-my-opencode
/// This ensures we sync the actual config file format (.jsonc or .json) being used
pub(super) fn resolve_dynamic_paths(mappings: Vec<FileMapping>) -> Vec<FileMapping> {
    mappings.into_iter().map(|mut mapping| {
        match mapping.id.as_str() {
            "opencode-main" => {
                // Use dynamic path detection for OpenCode main config
                if let Ok(actual_path) = open_code::get_default_config_path() {
                    // Extract filename from the actual path
                    if let Some(filename) = std::path::Path::new(&actual_path).file_name() {
                        let filename_str = filename.to_string_lossy();
                        mapping.windows_path = actual_path.clone();
                        mapping.wsl_path = format!("~/.config/opencode/{}", filename_str);
                    }
                }
            }
            "opencode-oh-my" => {
                // Use dynamic path detection for Oh My OpenCode config
                if let Ok(actual_path) = oh_my_opencode::get_oh_my_opencode_config_path() {
                    if let Some(filename) = actual_path.file_name() {
                        let filename_str = filename.to_string_lossy();
                        mapping.windows_path = actual_path.to_string_lossy().to_string();
                        mapping.wsl_path = format!("~/.config/opencode/{}", filename_str);
                    }
                }
            }
            "opencode-oh-my-slim" => {
                // Use dynamic path detection for Oh My OpenCode Slim config
                if let Ok(actual_path) = oh_my_opencode_slim::get_oh_my_opencode_slim_config_path() {
                    if let Some(filename) = actual_path.file_name() {
                        let filename_str = filename.to_string_lossy();
                        mapping.windows_path = actual_path.to_string_lossy().to_string();
                        mapping.wsl_path = format!("~/.config/opencode/{}", filename_str);
                    }
                }
            }
            _ => {}
        }
        mapping
    }).collect()
}

/// Update sync status in database
pub(super) async fn update_sync_status(
    state: &DbState,
    result: &SyncResult,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let (status, error) = if result.success {
        ("success".to_string(), None)
    } else {
        let error_msg = result.errors.join("; ");
        ("error".to_string(), Some(error_msg))
    };

    let now = Local::now().to_rfc3339();

    db.query("UPDATE wsl_sync_config SET last_sync_time = $time, last_sync_status = $status, last_sync_error = $error WHERE id = wsl_sync_config:`config`")
        .bind(("time", now))
        .bind(("status", status))
        .bind(("error", error))
        .await
        .map_err(|e| format!("Failed to update sync status: {}", e))?;

    Ok(())
}

/// Get default file mappings
pub fn default_file_mappings() -> Vec<FileMapping> {
    vec![
        // OpenCode
        FileMapping {
            id: "opencode-main".to_string(),
            name: "OpenCode 主配置".to_string(),
            module: "opencode".to_string(),
            windows_path: r"%USERPROFILE%\.config\opencode\opencode.jsonc".to_string(),
            wsl_path: "~/.config/opencode/opencode.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        FileMapping {
            id: "opencode-oh-my".to_string(),
            name: "Oh My OpenCode 配置".to_string(),
            module: "opencode".to_string(),
            windows_path: r"%USERPROFILE%\.config\opencode\oh-my-opencode.jsonc".to_string(),
            wsl_path: "~/.config/opencode/oh-my-opencode.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        FileMapping {
            id: "opencode-oh-my-slim".to_string(),
            name: "Oh My OpenCode Slim 配置".to_string(),
            module: "opencode".to_string(),
            windows_path: r"%USERPROFILE%\.config\opencode\oh-my-opencode-slim.json".to_string(),
            wsl_path: "~/.config/opencode/oh-my-opencode-slim.json".to_string(),
            enabled: false, // Disabled by default: this file is optional and not present on all systems
            is_pattern: false,
            is_directory: false,
        },
        FileMapping {
            id: "opencode-auth".to_string(),
            name: "OpenCode 认证信息".to_string(),
            module: "opencode".to_string(),
            windows_path: r"%USERPROFILE%\.local\share\opencode\auth.json".to_string(),
            wsl_path: "~/.local/share/opencode/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        FileMapping {
            id: "opencode-plugins".to_string(),
            name: "OpenCode 插件文件".to_string(),
            module: "opencode".to_string(),
            windows_path: r"%USERPROFILE%\.config\opencode\*.mjs".to_string(),
            wsl_path: "~/.config/opencode/".to_string(),
            enabled: true,
            is_pattern: true,
            is_directory: false,
        },
        // ClaudeCode
        FileMapping {
            id: "claude-settings".to_string(),
            name: "Claude Code 设置".to_string(),
            module: "claude".to_string(),
            windows_path: r"%USERPROFILE%\.claude\settings.json".to_string(),
            wsl_path: "~/.claude/settings.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        FileMapping {
            id: "claude-config".to_string(),
            name: "Claude Code 配置".to_string(),
            module: "claude".to_string(),
            windows_path: r"%USERPROFILE%\.claude\config.json".to_string(),
            wsl_path: "~/.claude/config.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        // Codex
        FileMapping {
            id: "codex-auth".to_string(),
            name: "Codex 认证".to_string(),
            module: "codex".to_string(),
            windows_path: r"%USERPROFILE%\.codex\auth.json".to_string(),
            wsl_path: "~/.codex/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        FileMapping {
            id: "codex-config".to_string(),
            name: "Codex 配置".to_string(),
            module: "codex".to_string(),
            windows_path: r"%USERPROFILE%\.codex\config.toml".to_string(),
            wsl_path: "~/.codex/config.toml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
    ]
}
