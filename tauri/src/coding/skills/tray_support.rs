//! Skills Tray Support Module
//!
//! Provides standardized API for tray menu integration.
//! This module handles all data fetching and processing for skills tray menu display.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::adapter::parse_sync_details;
use super::skill_store;
use super::sync_engine::{remove_path, sync_dir_for_tool_with_overwrite};
use super::tool_adapters::{
    get_all_tool_adapters, is_tool_installed, resolve_runtime_skills_path, runtime_adapter_by_key,
};
use super::types::{now_ms, SkillTarget};
use crate::DbState;

/// Item for tool selection in skill submenu
#[derive(Debug, Clone)]
pub struct TraySkillToolItem {
    /// Tool key (e.g. "claude_code")
    pub tool_key: String,
    /// Display name (e.g. "Claude Code")
    pub display_name: String,
    /// Whether this skill is synced to this tool
    pub is_synced: bool,
    /// Whether the tool is installed
    pub is_installed: bool,
}

/// Item for skill in tray menu
#[derive(Debug, Clone)]
pub struct TraySkillItem {
    /// Skill ID
    pub id: String,
    /// Skill name for display
    pub display_name: String,
    /// Central path for sync operations
    pub central_path: String,
    /// List of available tools with sync status
    pub tools: Vec<TraySkillToolItem>,
}

/// Data for skills section in tray menu
#[derive(Debug, Clone)]
pub struct TraySkillData {
    /// Title of the skills section
    pub title: String,
    /// List of managed skills
    pub items: Vec<TraySkillItem>,
}

/// Check if skills should be shown in tray menu
/// Reads the show_skills_in_tray setting
pub async fn is_skills_enabled_for_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.state::<DbState>();
    let raw = skill_store::get_setting(&state, "show_skills_in_tray")
        .await
        .ok()
        .flatten();
    match raw {
        Some(s) => s == "true",
        None => false,
    }
}

/// Get skills tray data
/// Returns all managed skills with their tool sync states
pub async fn get_skills_tray_data<R: Runtime>(app: &AppHandle<R>) -> Result<TraySkillData, String> {
    let state = app.state::<DbState>();

    // Get custom tools for adapter lookup
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();
    let all_adapters = get_all_tool_adapters(&custom_tools);

    // Get preferred tools (or use all installed tools if not set)
    let preferred_tools_raw = skill_store::get_setting(&state, "preferred_tools_v1")
        .await
        .ok()
        .flatten();
    let preferred_tools: Option<Vec<String>> =
        preferred_tools_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

    // Determine which tools to show
    let tools_to_show: Vec<String> = if let Some(pt) = preferred_tools {
        if !pt.is_empty() {
            pt
        } else {
            // Empty preferred tools means show all installed
            all_adapters
                .iter()
                .filter(|a| is_tool_installed(a).unwrap_or(false))
                .map(|a| a.key.clone())
                .collect()
        }
    } else {
        // No preferred tools set, show all installed
        all_adapters
            .iter()
            .filter(|a| is_tool_installed(a).unwrap_or(false))
            .map(|a| a.key.clone())
            .collect()
    };

    // Get all managed skills
    let skills = skill_store::get_managed_skills(&state).await?;

    let mut items: Vec<TraySkillItem> = Vec::new();

    for skill in skills {
        // Parse sync_details to get current targets
        let targets = parse_sync_details(&skill);
        let synced_tools: std::collections::HashSet<String> =
            targets.iter().map(|t| t.tool.clone()).collect();

        // Build tool items for this skill
        let tool_items: Vec<TraySkillToolItem> = tools_to_show
            .iter()
            .filter_map(|tool_key| {
                let adapter = all_adapters.iter().find(|a| a.key == *tool_key)?;
                let is_installed = is_tool_installed(adapter).unwrap_or(false);
                Some(TraySkillToolItem {
                    tool_key: tool_key.clone(),
                    display_name: adapter.display_name.clone(),
                    is_synced: synced_tools.contains(tool_key),
                    is_installed,
                })
            })
            .collect();

        items.push(TraySkillItem {
            id: skill.id,
            display_name: skill.name,
            central_path: skill.central_path,
            tools: tool_items,
        });
    }

    Ok(TraySkillData {
        title: "──── Skills ────".to_string(),
        items,
    })
}

/// Apply skill tool toggle from tray menu
/// Toggles the sync state of a skill for a specific tool
pub async fn apply_skills_tool_toggle<R: Runtime>(
    app: &AppHandle<R>,
    skill_id: &str,
    tool_key: &str,
) -> Result<(), String> {
    let state = app.state::<DbState>();

    // Get custom tools for adapter lookup
    let custom_tools = skill_store::get_custom_tools(&state)
        .await
        .unwrap_or_default();

    // Get skill by ID
    let skill = skill_store::get_skill_by_id(&state, skill_id)
        .await?
        .ok_or_else(|| format!("Skill not found: {}", skill_id))?;

    // Get runtime adapter for the tool
    let runtime_adapter = runtime_adapter_by_key(tool_key, &custom_tools)
        .ok_or_else(|| format!("Unknown tool: {}", tool_key))?;

    // Check if tool is installed
    if !runtime_adapter.is_custom && !is_tool_installed(&runtime_adapter).unwrap_or(false) {
        return Err(format!("Tool not installed: {}", tool_key));
    }

    // Check current sync state
    let existing_target = skill_store::get_skill_target(&state, skill_id, tool_key).await?;

    if existing_target.is_some() {
        // Currently synced -> unsync
        if let Some(target) = existing_target {
            // Remove the link/copy in tool directory
            remove_path(&target.target_path)?;
            skill_store::delete_skill_target(&state, skill_id, tool_key).await?;
        }
    } else {
        // Currently not synced -> sync
        let tool_root =
            resolve_runtime_skills_path(&runtime_adapter).map_err(|e| format!("{:#}", e))?;
        let target = tool_root.join(&skill.name);

        // Sync with overwrite (tray menu operates quickly, no confirmation dialog)
        let result = sync_dir_for_tool_with_overwrite(
            tool_key,
            std::path::Path::new(&skill.central_path),
            &target,
            true, // overwrite
            runtime_adapter.force_copy,
        )
        .map_err(|e| format!("{:#}", e))?;

        // Save target record
        let record = SkillTarget {
            tool: tool_key.to_string(),
            target_path: result.target_path.to_string_lossy().to_string(),
            mode: result.mode_used.as_str().to_string(),
            status: "ok".to_string(),
            error_message: None,
            synced_at: Some(now_ms()),
        };
        skill_store::upsert_skill_target(&state, skill_id, &record).await?;
    }

    // Notify frontend to refresh skills data
    let _ = app.emit("skills-changed", "tray");

    Ok(())
}
