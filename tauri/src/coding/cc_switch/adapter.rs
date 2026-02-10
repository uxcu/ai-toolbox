//! Data converters for importing CC-Switch configurations.
//!
//! Converts CC-Switch export structures into AI Toolbox input DTOs.

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::types::{CcSwitchMcpServer, CcSwitchProvider};
use crate::coding::claude_code::types::ClaudeCodeProviderInput;
use crate::coding::codex::types::CodexProviderInput;
use crate::coding::mcp::types::CreateMcpServerInput;

/// Convert a CC-Switch provider into Claude provider input.
pub fn convert_to_claude_provider(
    cc_provider: &CcSwitchProvider,
) -> Result<ClaudeCodeProviderInput, String> {
    let name = cc_provider.name.trim();
    if name.is_empty() {
        return Err(format!(
            "Failed to convert Claude provider: provider id '{}' has empty name",
            cc_provider.id
        ));
    }

    let settings_config = serde_json::to_string(&cc_provider.settings_config).map_err(|e| {
        format!(
            "Failed to convert Claude provider '{}': serialize settings_config failed: {}",
            cc_provider.name, e
        )
    })?;

    Ok(ClaudeCodeProviderInput {
        id: None,
        name: name.to_string(),
        category: normalize_category(cc_provider.category.as_deref()),
        settings_config,
        source_provider_id: normalize_opt_str(Some(cc_provider.id.as_str())),
        website_url: extract_website_url(&cc_provider.settings_config.extra),
        notes: normalize_opt_str(cc_provider.notes.as_deref()),
        icon: normalize_opt_str(cc_provider.icon.as_deref()),
        icon_color: normalize_opt_str(cc_provider.icon_color.as_deref()),
        sort_index: cc_provider.sort_index,
    })
}

/// Convert a CC-Switch provider into Codex provider input.
pub fn convert_to_codex_provider(
    cc_provider: &CcSwitchProvider,
) -> Result<CodexProviderInput, String> {
    let name = cc_provider.name.trim();
    if name.is_empty() {
        return Err(format!(
            "Failed to convert Codex provider: provider id '{}' has empty name",
            cc_provider.id
        ));
    }

    let settings_config = serde_json::to_string(&cc_provider.settings_config).map_err(|e| {
        format!(
            "Failed to convert Codex provider '{}': serialize settings_config failed: {}",
            cc_provider.name, e
        )
    })?;

    Ok(CodexProviderInput {
        id: None,
        name: name.to_string(),
        category: normalize_category(cc_provider.category.as_deref()),
        settings_config,
        source_provider_id: normalize_opt_str(Some(cc_provider.id.as_str())),
        website_url: extract_website_url(&cc_provider.settings_config.extra),
        notes: normalize_opt_str(cc_provider.notes.as_deref()),
        icon: normalize_opt_str(cc_provider.icon.as_deref()),
        icon_color: normalize_opt_str(cc_provider.icon_color.as_deref()),
        sort_index: cc_provider.sort_index,
        is_disabled: cc_provider.is_active.map(|active| !active),
    })
}

/// Convert a CC-Switch MCP server into MCP create input.
pub fn convert_to_mcp_server(cc_mcp: &CcSwitchMcpServer) -> Result<CreateMcpServerInput, String> {
    let name = cc_mcp.name.trim();
    if name.is_empty() {
        return Err(format!(
            "Failed to convert MCP server: server id '{}' has empty name",
            cc_mcp.id
        ));
    }

    let server_type = normalize_server_type(&cc_mcp.server_type)
        .map_err(|e| format!("Failed to convert MCP server '{}': {}", cc_mcp.name, e))?;

    Ok(CreateMcpServerInput {
        name: name.to_string(),
        server_type,
        server_config: cc_mcp.server_config.clone(),
        enabled_tools: extract_enabled_tools(&cc_mcp.apps),
        description: normalize_opt_str(cc_mcp.description.as_deref()),
        tags: cc_mcp
            .tags
            .iter()
            .filter_map(|t| {
                let trimmed = t.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
            .collect(),
    })
}

/// Extract enabled tool names from CC-Switch `apps` payload.
pub fn extract_enabled_tools(apps: &Option<Value>) -> Vec<String> {
    let mut tools = Vec::new();

    let Some(obj) = apps.as_ref().and_then(|v| v.as_object()) else {
        return tools;
    };

    for (tool_name, enabled) in obj {
        let is_enabled = enabled.as_bool().unwrap_or_else(|| {
            enabled
                .as_object()
                .and_then(|o| o.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });

        if is_enabled {
            tools.push(tool_name.to_string());
        }
    }

    tools.sort();
    tools
}

/// Convert an ISO 8601 timestamp into Unix milliseconds.
pub fn iso_to_unix_ms(iso: &str) -> i64 {
    DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|e| {
            eprintln!("Failed to parse ISO timestamp '{}': {}", iso, e);
            Utc::now().timestamp_millis()
        })
}

fn normalize_category(category: Option<&str>) -> String {
    category
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("imported")
        .to_string()
}

fn normalize_opt_str(value: Option<&str>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn extract_website_url(extra: &std::collections::HashMap<String, Value>) -> Option<String> {
    extra
        .get("website_url")
        .or_else(|| extra.get("websiteUrl"))
        .and_then(|v| v.as_str())
        .and_then(|v| {
            let trimmed = v.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn normalize_server_type(server_type: &str) -> Result<String, String> {
    match server_type.trim().to_lowercase().as_str() {
        "stdio" => Ok("stdio".to_string()),
        "http" => Ok("http".to_string()),
        "sse" => Ok("sse".to_string()),
        other => Err(format!(
            "unsupported server_type '{}' (expected: stdio/http/sse)",
            other
        )),
    }
}
