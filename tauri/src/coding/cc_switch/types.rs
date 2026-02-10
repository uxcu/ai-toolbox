//! Type definitions for CC-Switch configuration import.
//!
//! This module defines the JSON structures exported by CC-Switch and
//! the DTO types returned by AI Toolbox import commands.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Root structure of a CC-Switch exported JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcSwitchExport {
    /// CC-Switch export format version.
    pub version: String,
    /// Export timestamp in ISO 8601 format.
    #[serde(default)]
    pub export_time: Option<String>,
    /// Provider collections grouped by application type.
    pub providers: CcSwitchProviders,
    /// MCP server map keyed by server identifier.
    #[serde(default)]
    pub mcp_servers: HashMap<String, CcSwitchMcpServer>,
}

/// Provider collections in CC-Switch export format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CcSwitchProviders {
    /// Claude provider list.
    #[serde(default)]
    pub claude: Vec<CcSwitchProvider>,
    /// Codex provider list.
    #[serde(default)]
    pub codex: Vec<CcSwitchProvider>,
    /// Gemini provider list.
    #[serde(default)]
    pub gemini: Vec<CcSwitchProvider>,
}

/// Provider record in CC-Switch export format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcSwitchProvider {
    /// Unique provider ID.
    pub id: String,
    /// Provider display name.
    pub name: String,
    /// Provider category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Provider settings payload.
    pub settings_config: CcSwitchSettingsConfig,
    /// Whether provider is currently active in CC-Switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
    /// Optional provider note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Optional provider icon key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Optional icon color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    /// Optional sort order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i32>,
}

/// Nested provider settings configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CcSwitchSettingsConfig {
    /// Environment variable map used by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
    /// Additional config fields preserved for forward compatibility.
    #[serde(flatten, default)]
    pub extra: HashMap<String, Value>,
}

/// MCP server record in CC-Switch export format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcSwitchMcpServer {
    /// Unique server ID.
    pub id: String,
    /// Server display name.
    pub name: String,
    /// Transport type such as `stdio`, `http`, or `sse`.
    pub server_type: String,
    /// Transport-specific server configuration.
    pub server_config: Value,
    /// App enablement map, e.g. `{ "claude": true }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<Value>,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional server tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Provider import statistics grouped by app type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchProviderStats {
    /// Number of imported Claude providers.
    pub claude: i32,
    /// Number of imported Codex providers.
    pub codex: i32,
    /// Number of imported Gemini providers.
    pub gemini: i32,
}

/// Result returned by CC-Switch import command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchImportResult {
    /// Whether the import is considered successful.
    pub success: bool,
    /// Summary message for UI display.
    pub message: String,
    /// Imported provider statistics.
    pub provider_stats: CcSwitchProviderStats,
    /// Imported MCP server count.
    pub mcp_count: i32,
    /// Number of skipped providers due to duplicate names.
    #[serde(default)]
    pub providers_skipped: i32,
    /// Number of skipped MCP servers due to duplicate names.
    #[serde(default)]
    pub mcp_servers_skipped: i32,
    /// Error list collected during import.
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Preview payload returned by CC-Switch preview command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CcSwitchPreview {
    /// CC-Switch export format version.
    pub version: String,
    /// Export timestamp in ISO 8601 format.
    #[serde(default)]
    pub export_time: Option<String>,
    /// Provider statistics in the export payload.
    #[serde(default)]
    pub provider_stats: CcSwitchProviderStats,
    /// MCP server count in the export payload.
    #[serde(default)]
    pub mcp_count: i32,
    /// Total provider count in the export payload.
    #[serde(default)]
    pub provider_count: i32,
}

/// Installation detection info for local CC-Switch runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CcSwitchInstallationInfo {
    pub installed: bool,
    pub config_dir: Option<String>,
    pub version: Option<String>,
    pub last_export: Option<String>,
}

/// Options accepted by CC-Switch import command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    /// Import Claude providers.
    #[serde(default = "default_true")]
    pub import_claude: bool,
    /// Import Codex providers.
    #[serde(default = "default_true")]
    pub import_codex: bool,
    /// Import MCP servers.
    #[serde(default = "default_true")]
    pub import_mcp: bool,
    /// Skip duplicates when true.
    #[serde(default)]
    pub skip_duplicates: bool,
}

fn default_true() -> bool {
    true
}
