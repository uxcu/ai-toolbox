use indexmap::IndexMap;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tauri::Emitter;

use super::adapter;
use super::types::*;
use crate::db::DbState;

// ============================================================================
// Helper Functions
// ============================================================================

/// Fields in model that should be removed if they are empty objects
const MODEL_EMPTY_OBJECT_FIELDS: &[&str] = &["options", "variants", "modalities"];

/// Recursively clean empty objects from the config
/// Specifically targets options, variants, modalities in models
fn clean_empty_objects(value: &mut Value) {
    if let Value::Object(map) = value {
        // Check if this is a provider section
        if let Some(Value::Object(providers)) = map.get_mut("provider") {
            for (_provider_key, provider_value) in providers.iter_mut() {
                if let Value::Object(provider) = provider_value {
                    // Check models in each provider
                    if let Some(Value::Object(models)) = provider.get_mut("models") {
                        for (_model_key, model_value) in models.iter_mut() {
                            if let Value::Object(model) = model_value {
                                // Remove empty object fields
                                for field in MODEL_EMPTY_OBJECT_FIELDS {
                                    if let Some(Value::Object(obj)) = model.get(*field) {
                                        if obj.is_empty() {
                                            model.remove(*field);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// OpenCode Commands
// ============================================================================

/// Get OpenCode config file path with priority: common config > system env > shell config > default
#[tauri::command]
pub async fn get_opencode_config_path(state: tauri::State<'_, DbState>) -> Result<String, String> {
    // 1. Check common config (highest priority)
    if let Some(common_config) = get_opencode_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(custom_path);
            }
        }
    }

    // 2. Check system environment variable (second priority)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Ok(env_path);
        }
    }

    // 3. Check shell configuration files (third priority)
    if let Some(shell_path) = super::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Ok(shell_path);
        }
    }

    // 4. Return default path
    get_default_config_path()
}

/// Get OpenCode config path info including source
#[tauri::command]
pub async fn get_opencode_config_path_info(
    state: tauri::State<'_, DbState>,
) -> Result<ConfigPathInfo, String> {
    // 1. Check common config (highest priority)
    if let Some(common_config) = get_opencode_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(ConfigPathInfo {
                    path: custom_path,
                    source: "custom".to_string(),
                });
            }
        }
    }

    // 2. Check system environment variable (second priority)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Ok(ConfigPathInfo {
                path: env_path,
                source: "env".to_string(),
            });
        }
    }

    // 3. Check shell configuration files (third priority)
    if let Some(shell_path) = super::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Ok(ConfigPathInfo {
                path: shell_path,
                source: "shell".to_string(),
            });
        }
    }

    // 4. Return default path
    let default_path = get_default_config_path()?;
    Ok(ConfigPathInfo {
        path: default_path,
        source: "default".to_string(),
    })
}

/// Helper function to get default config path
/// Returns the actual config file path (checks .jsonc first, then .json)
pub fn get_default_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_dir = Path::new(&home_dir).join(".config").join("opencode");

    // Check for .jsonc first, then .json
    let jsonc_path = config_dir.join("opencode.jsonc");
    let json_path = config_dir.join("opencode.json");

    if jsonc_path.exists() {
        Ok(jsonc_path.to_string_lossy().to_string())
    } else if json_path.exists() {
        Ok(json_path.to_string_lossy().to_string())
    } else {
        // Return default path for new file
        Ok(jsonc_path.to_string_lossy().to_string())
    }
}

/// Read OpenCode configuration file with detailed result
#[tauri::command]
pub async fn read_opencode_config(
    state: tauri::State<'_, DbState>,
) -> Result<ReadConfigResult, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Ok(ReadConfigResult::NotFound {
            path: config_path_str,
        });
    }

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ReadConfigResult::Error {
                error: format!("Failed to read config file: {}", e),
            })
        }
    };

    match json5::from_str::<OpenCodeConfig>(&content) {
        Ok(mut config) => {
            // Initialize provider if missing
            if config.provider.is_none() {
                config.provider = Some(IndexMap::<String, OpenCodeProvider>::new());
            }

            // Fill missing name fields with provider key
            // Fill missing npm fields with smart default based on provider key/name
            if let Some(ref mut providers) = config.provider {
                for (key, provider) in providers.iter_mut() {
                    if provider.name.is_none() {
                        provider.name = Some(key.clone());
                    }
                    if provider.npm.is_none() {
                        // Smart npm inference based on provider key or name (case-insensitive)
                        let key_lower = key.to_lowercase();
                        let name_lower = provider
                            .name
                            .as_ref()
                            .map(|n| n.to_lowercase())
                            .unwrap_or_default();

                        let inferred_npm = if key_lower.contains("google")
                            || key_lower.contains("gemini")
                            || name_lower.contains("google")
                            || name_lower.contains("gemini")
                        {
                            "@ai-sdk/google"
                        } else if key_lower.contains("anthropic")
                            || key_lower.contains("claude")
                            || name_lower.contains("anthropic")
                            || name_lower.contains("claude")
                        {
                            "@ai-sdk/anthropic"
                        } else {
                            "@ai-sdk/openai-compatible"
                        };

                        provider.npm = Some(inferred_npm.to_string());
                    }
                }
            }

            Ok(ReadConfigResult::Success { config })
        }
        Err(e) => {
            // Truncate content preview to first 500 chars
            let preview = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            Ok(ReadConfigResult::ParseError {
                path: config_path_str,
                error: e.to_string(),
                content_preview: Some(preview),
            })
        }
    }
}

/// Backup OpenCode configuration file by renaming it with .bak.{timestamp} suffix
#[tauri::command]
pub async fn backup_opencode_config(state: tauri::State<'_, DbState>) -> Result<String, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("Config file does not exist".to_string());
    }

    // Generate backup path with timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_path_str = format!("{}.bak.{}", config_path_str, timestamp);
    let backup_path = Path::new(&backup_path_str);

    // Rename the file to backup
    fs::rename(config_path, backup_path)
        .map_err(|e| format!("Failed to backup config file: {}", e))?;

    Ok(backup_path_str.to_string())
}

/// Save OpenCode configuration file
#[tauri::command]
pub async fn save_opencode_config<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle<R>,
    config: OpenCodeConfig,
) -> Result<(), String> {
    apply_config_internal(state, &app, config, false).await
}

/// Internal function to save config and emit events
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, DbState>,
    app: &tauri::AppHandle<R>,
    config: OpenCodeConfig,
    from_tray: bool,
) -> Result<(), String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
    }

    // Serialize to JSON Value first, then clean up empty objects
    let mut json_value =
        serde_json::to_value(&config).map_err(|e| format!("Failed to serialize config: {}", e))?;

    // Clean up empty objects in models (options, variants, modalities)
    clean_empty_objects(&mut json_value);

    // Serialize with pretty printing
    let json_content = serde_json::to_string_pretty(&json_value)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(config_path, json_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    Ok(())
}

// ============================================================================
// OpenCode Common Config Commands
// ============================================================================

/// Get OpenCode common config
#[tauri::command]
pub async fn get_opencode_common_config(
    state: tauri::State<'_, DbState>,
) -> Result<Option<OpenCodeCommonConfig>, String> {
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_common_config:`common` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query opencode common config: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.first() {
                Ok(Some(adapter::from_db_value(record.clone())))
            } else {
                Ok(None)
            }
        }
        Err(e) => {
            // 反序列化失败，删除旧数据以修复版本冲突
            eprintln!(
                "⚠️ OpenCode common config has incompatible format, cleaning up: {}",
                e
            );
            let _ = db.query("DELETE opencode_common_config:`common`").await;
            Ok(None)
        }
    }
}

/// Save OpenCode common config
#[tauri::command]
pub async fn save_opencode_common_config(
    state: tauri::State<'_, DbState>,
    config: OpenCodeCommonConfig,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let json_data = adapter::to_db_value(&config);

    // Use UPSERT to handle both update and create
    db.query("UPSERT opencode_common_config:`common` CONTENT $data")
        .bind(("data", json_data))
        .await
        .map_err(|e| format!("Failed to save opencode common config: {}", e))?;

    Ok(())
}

// ============================================================================
// Free Models Commands
// ============================================================================

/// Get OpenCode free models from opencode channel
/// Returns free models where cost.input and cost.output are both 0
#[tauri::command]
pub async fn get_opencode_free_models(
    state: tauri::State<'_, DbState>,
    force_refresh: Option<bool>,
) -> Result<GetFreeModelsResponse, String> {
    let (free_models, from_cache, updated_at) =
        super::free_models::get_free_models(&state, force_refresh.unwrap_or(false)).await?;
    let total = free_models.len();

    Ok(GetFreeModelsResponse {
        free_models,
        total,
        from_cache,
        updated_at,
    })
}

/// Get provider models data by provider_id
/// Returns the complete model information for a specific provider
#[tauri::command]
pub async fn get_provider_models(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<Option<ProviderModelsData>, String> {
    super::free_models::get_provider_models_internal(&state, &provider_id).await
}

// ============================================================================
// Unified Models Commands
// ============================================================================

/// Get unified model list combining custom providers and official providers from auth.json
/// Returns all available models sorted by display name
#[tauri::command]
pub async fn get_opencode_unified_models(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<UnifiedModelOption>, String> {
    // Read auth.json to get official provider ids
    let auth_channels = super::free_models::read_auth_channels();

    // Read config to get custom providers
    let result = read_opencode_config(state.clone()).await?;
    let custom_providers = match result {
        ReadConfigResult::Success { config } => config.provider,
        _ => None,
    };

    // Get unified model list
    let models =
        super::free_models::get_unified_models(&state, custom_providers.as_ref(), &auth_channels)
            .await;

    Ok(models)
}

// ============================================================================
// Official Auth Providers Commands
// ============================================================================

/// Get official auth providers data from auth.json
/// Returns providers split into standalone (not in custom config) and merged (models only)
#[tauri::command]
pub async fn get_opencode_auth_providers(
    state: tauri::State<'_, DbState>,
) -> Result<GetAuthProvidersResponse, String> {
    // Read config to get custom providers
    let result = read_opencode_config(state.clone()).await?;
    let custom_providers = match result {
        ReadConfigResult::Success { config } => config.provider,
        _ => None,
    };

    // Get auth providers data
    let response =
        super::free_models::get_auth_providers_data(&state, custom_providers.as_ref()).await;

    Ok(response)
}

// ============================================================================
// Favorite Plugin Commands
// ============================================================================

/// Default favorite plugins to initialize on first use
const DEFAULT_FAVORITE_PLUGINS: &[&str] = &[
    "oh-my-opencode@latest",
    "oh-my-opencode-slim",
    "opencode-antigravity-auth",
    "opencode-openai-codex-auth",
    "opencode-omit-max-tokens",
    "opencode-axonhub-tracing",
];

const FEISHU_BRIDGE_PLUGIN_FILE: &str = "opencode-feishu-ws-bridge.mjs";
const FEISHU_BRIDGE_SERVER_FILE: &str = "server.mjs";
const FEISHU_BRIDGE_PKG_FILE: &str = "package.json";
const FEISHU_BRIDGE_ENV_EXAMPLE_FILE: &str = ".env.example";
const FEISHU_BRIDGE_README_FILE: &str = "README.md";

const FEISHU_BRIDGE_PLUGIN_TEMPLATE: &str = r#"
const DEFAULT_WS_URL = process.env.FEISHU_BRIDGE_WS_URL || "ws://127.0.0.1:17171/opencode";
const RECONNECT_MS = 2000;
const FORWARD_EVENT_TYPES = new Set([
  "message.updated",
  "permission.updated",
  "session.idle",
  "session.error",
  "session.updated",
]);

let socket = null;
let reconnectTimer = null;

function connect() {
  if (socket && (socket.readyState === 0 || socket.readyState === 1)) {
    return;
  }

  socket = new WebSocket(DEFAULT_WS_URL);

  socket.addEventListener("open", () => {
    console.log("[feishu-bridge] connected", DEFAULT_WS_URL);
    send({
      type: "bridge.hello",
      source: "opencode-plugin",
      ts: Date.now(),
    });
  });

  socket.addEventListener("close", () => {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
    }
    reconnectTimer = setTimeout(connect, RECONNECT_MS);
  });

  socket.addEventListener("error", (error) => {
    console.error("[feishu-bridge] websocket error", error);
  });
}

function send(payload) {
  if (!socket || socket.readyState !== 1) {
    return;
  }

  try {
    socket.send(JSON.stringify(payload));
  } catch (error) {
    console.error("[feishu-bridge] send error", error);
  }
}

const feishuBridgePlugin = async function feishuBridgePlugin() {
  connect();

  return {
    event: async ({ event }) => {
      if (!event || !FORWARD_EVENT_TYPES.has(event.type)) {
        return;
      }

      send({
        type: "opencode.event",
        event,
        ts: Date.now(),
      });
    },
  };
};

export default feishuBridgePlugin;
export { feishuBridgePlugin };
"#;

const FEISHU_BRIDGE_SERVER_TEMPLATE: &str = r#"
import * as lark from "@larksuiteoapi/node-sdk";
import { WebSocketServer } from "ws";
import { createOpencode } from "@opencode-ai/sdk";
import { Surreal } from "surrealdb";

const APP_ID = process.env.FEISHU_APP_ID || "";
const APP_SECRET = process.env.FEISHU_APP_SECRET || "";
const OPENCODE_BASE_URL = process.env.OPENCODE_SERVER_BASE_URL || "http://127.0.0.1:4096";
const WS_HOST = process.env.LOCAL_WS_HOST || "127.0.0.1";
const WS_PORT = Number(process.env.LOCAL_WS_PORT || 17171);
const BOT_NAME = process.env.FEISHU_BOT_NAME || "OpenCode";
const STREAM_UPDATE_INTERVAL_MS = Number(process.env.FEISHU_STREAM_UPDATE_INTERVAL_MS || 1200);

const SURREALDB_URL = process.env.SURREALDB_URL || "";
const SURREALDB_NAMESPACE = process.env.SURREALDB_NAMESPACE || "opencode_bridge";
const SURREALDB_DATABASE = process.env.SURREALDB_DATABASE || "feishu";
const SURREALDB_USER = process.env.SURREALDB_USER || "root";
const SURREALDB_PASS = process.env.SURREALDB_PASS || "root";
const SURREALDB_SESSION_TABLE = process.env.SURREALDB_SESSION_TABLE || "feishu_session";
const SURREALDB_MESSAGE_TABLE = process.env.SURREALDB_MESSAGE_TABLE || "feishu_message";

const ALLOWED_CHAT_IDS = new Set(
  (process.env.FEISHU_ALLOWED_CHAT_IDS || "")
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean),
);

if (!APP_ID || !APP_SECRET) {
  console.error("Missing FEISHU_APP_ID or FEISHU_APP_SECRET");
  process.exit(1);
}

const chatSessions = new Map(); // chatID -> sessionID
const sessionChats = new Map(); // sessionID -> chatID
const activeStreams = new Map(); // sessionID -> { chatID, messageID, text, updatedAt }

const opencode = await createOpencode({ baseURL: OPENCODE_BASE_URL });
const surreal = new Surreal();
let surrealReady = false;

const CARD_COLORS = {
  thinking: "blue",
  done: "green",
  error: "red",
};

function shortId(id) {
  if (!id) return "";
  return String(id).slice(0, 8);
}

function normalizeText(text) {
  if (!text) return "";
  return String(text)
    .replace(/\r/g, "")
    .replace(/\u0000/g, "")
    .trim();
}

function clampText(text, maxLength = 3500) {
  const normalized = normalizeText(text);
  if (normalized.length <= maxLength) {
    return normalized;
  }
  return `${normalized.slice(0, maxLength)}...`;
}

function deepCollectText(node, out) {
  if (!node) return;
  if (typeof node === "string") {
    out.push(node);
    return;
  }
  if (Array.isArray(node)) {
    for (const item of node) deepCollectText(item, out);
    return;
  }
  if (typeof node === "object") {
    if (node.type === "text" && typeof node.text === "string") {
      out.push(node.text);
    }
    if (typeof node.content === "string") out.push(node.content);
    if (typeof node.delta === "string") out.push(node.delta);
    if (typeof node.output === "string") out.push(node.output);
    if (node.message) deepCollectText(node.message, out);
    if (node.part) deepCollectText(node.part, out);
    if (node.parts) deepCollectText(node.parts, out);
    if (node.data) deepCollectText(node.data, out);
    if (node.payload) deepCollectText(node.payload, out);
  }
}

function extractEventText(event) {
  const chunks = [];
  deepCollectText(event, chunks);
  return normalizeText(chunks.join("\n"));
}

function renderCard({ title, status, body, sessionID, note }) {
  return {
    config: { wide_screen_mode: true, enable_forward: true },
    header: {
      template: CARD_COLORS[status] || "blue",
      title: { tag: "plain_text", content: title },
    },
    elements: [
      {
        tag: "div",
        text: {
          tag: "lark_md",
          content: `**Session:** \`${shortId(sessionID)}\`${note ? `\n${note}` : ""}`,
        },
      },
      { tag: "hr" },
      {
        tag: "div",
        text: {
          tag: "lark_md",
          content: body || "(empty)",
        },
      },
    ],
  };
}

function toRecordId(prefix, value) {
  const safe = String(value || "")
    .toLowerCase()
    .replace(/[^a-z0-9_-]/g, "_")
    .slice(0, 120);
  return `${prefix}:${safe || "unknown"}`;
}

async function initSurreal() {
  if (!SURREALDB_URL) {
    console.warn("[feishu-bridge] SURREALDB_URL is empty, persistence disabled");
    return;
  }
  try {
    await surreal.connect(SURREALDB_URL);
    await surreal.signin({ username: SURREALDB_USER, password: SURREALDB_PASS });
    await surreal.use({ namespace: SURREALDB_NAMESPACE, database: SURREALDB_DATABASE });
    surrealReady = true;
    console.log("[feishu-bridge] surrealdb connected");
  } catch (error) {
    surrealReady = false;
    console.error("[feishu-bridge] surrealdb init failed, fallback to memory", error);
  }
}

async function loadSessionFromDb(chatID) {
  if (!surrealReady) return null;
  try {
    const record = await surreal.select(toRecordId(SURREALDB_SESSION_TABLE, chatID));
    return record?.session_id || null;
  } catch (error) {
    console.error("[feishu-bridge] load session failed", error);
    return null;
  }
}

async function saveSessionToDb(chatID, sessionID) {
  if (!surrealReady) return;
  try {
    await surreal.upsert(toRecordId(SURREALDB_SESSION_TABLE, chatID), {
      chat_id: chatID,
      session_id: sessionID,
      updated_at: new Date().toISOString(),
    });
  } catch (error) {
    console.error("[feishu-bridge] save session failed", error);
  }
}

async function saveMessageToDb({ chatID, sessionID, role, content, messageID }) {
  if (!surrealReady) return;
  try {
    await surreal.create(SURREALDB_MESSAGE_TABLE, {
      chat_id: chatID,
      session_id: sessionID,
      role,
      content: normalizeText(content),
      message_id: messageID || null,
      created_at: new Date().toISOString(),
    });
  } catch (error) {
    console.error("[feishu-bridge] save message failed", error);
  }
}

const wsServer = new WebSocketServer({
  host: WS_HOST,
  port: WS_PORT,
  path: "/opencode",
});

wsServer.on("connection", (ws) => {
  console.log("[feishu-bridge] local ws connected");

  ws.on("message", async (buf) => {
    try {
      const payload = JSON.parse(buf.toString());
      if (payload?.type !== "opencode.event") {
        return;
      }

      const event = payload?.event || {};
      const eventType = event?.type;
      const sessionID = event?.sessionID || event?.session_id;
      if (!eventType || !sessionID) {
        return;
      }

      const streamState = activeStreams.get(sessionID);
      if (!streamState) {
        return;
      }

      if (eventType === "session.error") {
        const errorText = extractEventText(event) || "session.error";
        await patchCard(streamState.chatID, streamState.messageID, renderCard({
          title: `${BOT_NAME} · Error`,
          status: "error",
          body: clampText(errorText, 3000),
          sessionID,
          note: "OpenCode session returned an error",
        }));
        await saveMessageToDb({
          chatID: streamState.chatID,
          sessionID,
          role: "assistant_error",
          content: errorText,
          messageID: streamState.messageID,
        });
        return;
      }

      const streamText = extractEventText(event);
      if (!streamText) {
        return;
      }

      streamState.text = streamText;
      const now = Date.now();
      if (now - streamState.updatedAt < STREAM_UPDATE_INTERVAL_MS) {
        return;
      }
      streamState.updatedAt = now;

      await patchCard(streamState.chatID, streamState.messageID, renderCard({
        title: `${BOT_NAME} · Streaming`,
        status: "thinking",
        body: clampText(streamState.text, 3000),
        sessionID,
        note: `event: ${eventType}`,
      }));
    } catch (error) {
      console.error("Failed to handle local ws message", error);
    }
  });
});

console.log(`[feishu-bridge] ws listening at ws://${WS_HOST}:${WS_PORT}/opencode`);

const larkClient = new lark.Client({
  appId: APP_ID,
  appSecret: APP_SECRET,
});

async function sendCard(chatID, card) {
  const response = await larkClient.im.message.create({
    params: { receive_id_type: "chat_id" },
    data: {
      receive_id: chatID,
      msg_type: "interactive",
      content: JSON.stringify(card),
    },
  });
  return response?.data?.message_id || response?.data?.messageId || "";
}

async function patchCard(chatID, messageID, card) {
  if (!messageID) {
    return;
  }
  try {
    await larkClient.im.message.patch({
      path: {
        message_id: messageID,
      },
      data: {
        content: JSON.stringify(card),
      },
    });
  } catch (error) {
    console.error("[feishu-bridge] patch card failed", error);
    await sendText(chatID, `[${BOT_NAME}] card update failed, fallback to text`);
    await sendText(chatID, clampText(card?.elements?.[2]?.text?.content || "(empty)", 2000));
  }
}

async function sendText(chatID, text) {
  await larkClient.im.message.create({
    params: { receive_id_type: "chat_id" },
    data: {
      receive_id: chatID,
      msg_type: "text",
      content: JSON.stringify({ text }),
    },
  });
}

function getMessageText(content) {
  if (!content) {
    return "";
  }
  try {
    const parsed = JSON.parse(content);
    return (parsed?.text || "").trim();
  } catch {
    return "";
  }
}

function summarizePromptResult(result) {
  if (!result) {
    return "(empty response)";
  }
  if (typeof result === "string") {
    return result;
  }
  if (result?.message?.parts?.length) {
    return result.message.parts
      .filter((part) => part?.type === "text")
      .map((part) => part.text || "")
      .join("\n")
      .trim();
  }
  if (result?.parts?.length) {
    return result.parts
      .filter((part) => part?.type === "text")
      .map((part) => part.text || "")
      .join("\n")
      .trim();
  }
  return JSON.stringify(result);
}

async function ensureSession(chatID) {
  const current = chatSessions.get(chatID);
  if (current) {
    return current;
  }

  const dbSession = await loadSessionFromDb(chatID);
  if (dbSession) {
    chatSessions.set(chatID, dbSession);
    sessionChats.set(dbSession, chatID);
    return dbSession;
  }

  const created = await opencode.session.create({
    title: `Feishu-${chatID}`,
  });

  const sessionID = created?.sessionID || created?.id;
  if (!sessionID) {
    throw new Error("Failed to create OpenCode session");
  }

  chatSessions.set(chatID, sessionID);
  sessionChats.set(sessionID, chatID);
  await saveSessionToDb(chatID, sessionID);
  return sessionID;
}

async function handleChatCommand(chatID, text) {
  const command = text.trim();
  if (!command) {
    return;
  }

  if (command === "/help") {
    await sendText(
      chatID,
      [
        `${BOT_NAME} commands:`,
        "/help - show help",
        "/new - create a new OpenCode session",
        "/status - show current session id",
        "/ask <prompt> - send prompt to OpenCode",
      ].join("\n"),
    );
    return;
  }

  if (command === "/new") {
    chatSessions.delete(chatID);
    const sessionID = await ensureSession(chatID);
    await saveSessionToDb(chatID, sessionID);
    await sendText(chatID, `[${BOT_NAME}] created a new session: ${sessionID}`);
    return;
  }

  if (command === "/status") {
    const sessionID = await ensureSession(chatID);
    await sendText(chatID, `[${BOT_NAME}] current session: ${sessionID}`);
    return;
  }

  const prompt = command.startsWith("/ask")
    ? command.slice(4).trim()
    : command;

  if (!prompt) {
    await sendText(chatID, `[${BOT_NAME}] please provide a prompt, e.g. /ask summarize today's code changes`);
    return;
  }

  const sessionID = await ensureSession(chatID);
  await saveMessageToDb({
    chatID,
    sessionID,
    role: "user",
    content: prompt,
  });

  const initialCard = renderCard({
    title: `${BOT_NAME} · Streaming`,
    status: "thinking",
    body: "OpenCode is thinking...",
    sessionID,
    note: "Starting stream...",
  });
  const messageID = await sendCard(chatID, initialCard);
  activeStreams.set(sessionID, {
    chatID,
    messageID,
    text: "",
    updatedAt: 0,
  });

  const response = await opencode.session.prompt(sessionID, {
    parts: [{ type: "text", text: prompt }],
  });

  const summary = summarizePromptResult(response) || "(empty response)";
  const streamState = activeStreams.get(sessionID);
  const finalText = streamState?.text || summary;

  const finalCard = renderCard({
    title: `${BOT_NAME} · Completed`,
    status: "done",
    body: clampText(finalText, 3000),
    sessionID,
    note: "Done",
  });

  if (streamState?.messageID) {
    await patchCard(chatID, streamState.messageID, finalCard);
  } else {
    await sendCard(chatID, finalCard);
  }

  await saveMessageToDb({
    chatID,
    sessionID,
    role: "assistant",
    content: finalText,
    messageID: streamState?.messageID || null,
  });

  activeStreams.delete(sessionID);
}

const eventDispatcher = new lark.ws.EventDispatcher({}).register({
  "im.message.receive_v1": async (data) => {
    const chatID = data?.event?.message?.chat_id;
    if (!chatID) {
      return;
    }

    if (ALLOWED_CHAT_IDS.size > 0 && !ALLOWED_CHAT_IDS.has(chatID)) {
      return;
    }

    const text = getMessageText(data?.event?.message?.content);
    if (!text) {
      return;
    }

    try {
      await handleChatCommand(chatID, text);
    } catch (error) {
      console.error("Failed to handle command", error);
      await sendText(chatID, `[${BOT_NAME}] failed: ${error?.message || String(error)}`);
    }
  },
});

const wsClient = new lark.ws.Client({
  appId: APP_ID,
  appSecret: APP_SECRET,
  eventDispatcher,
});

await initSurreal();
await wsClient.start();
console.log("[feishu-bridge] feishu ws started");
"#;

const FEISHU_BRIDGE_PACKAGE_TEMPLATE: &str = r#"
{
  "name": "opencode-feishu-ws-bridge-local",
  "private": true,
  "type": "module",
  "version": "0.2.0",
  "scripts": {
    "start": "node ./server.mjs"
  },
  "dependencies": {
    "@larksuiteoapi/node-sdk": "^1.54.0",
    "@opencode-ai/sdk": "latest",
    "surrealdb": "^1.3.2",
    "ws": "^8.18.0"
  }
}
"#;

const FEISHU_BRIDGE_ENV_EXAMPLE_TEMPLATE: &str = r#"
FEISHU_APP_ID=
FEISHU_APP_SECRET=
FEISHU_BOT_NAME=OpenCode
FEISHU_ALLOWED_CHAT_IDS=
OPENCODE_SERVER_BASE_URL=http://127.0.0.1:4096
LOCAL_WS_HOST=127.0.0.1
LOCAL_WS_PORT=17171
FEISHU_STREAM_UPDATE_INTERVAL_MS=1200
SURREALDB_URL=ws://127.0.0.1:8000/rpc
SURREALDB_NAMESPACE=opencode_bridge
SURREALDB_DATABASE=feishu
SURREALDB_USER=root
SURREALDB_PASS=root
SURREALDB_SESSION_TABLE=feishu_session
SURREALDB_MESSAGE_TABLE=feishu_message
"#;

const FEISHU_BRIDGE_README_TEMPLATE: &str = r#"
# OpenCode Feishu WS Bridge (Local)

This folder is generated by AI Toolbox.

## Features

- Stream OpenCode generation updates to Feishu interactive card
- Persist chat-session mapping to SurrealDB
- Persist user and assistant messages to SurrealDB

## Files

- `server.mjs`: Feishu long-connection bot + OpenCode bridge service
- `.env.example`: required environment variables
- `package.json`: dependencies and start script

## Start

1. Copy `.env.example` to `.env` and fill Feishu app credentials.
2. Prepare SurrealDB (recommended):

   ```bash
   surreal start --user root --pass root memory
   ```

3. Start OpenCode server:

   ```bash
   opencode serve --hostname 127.0.0.1 --port 4096
   ```

4. Install dependencies and run bridge:

   ```bash
   cd ~/.config/opencode/feishu-ws-bridge
   npm install
   node --env-file=.env ./server.mjs
   ```

5. Open Feishu chat with bot and send commands:

   - `/help`
   - `/new`
   - `/status`
   - `/ask your prompt`

## Local plugin

Plugin file path:

- `~/.config/opencode/plugins/opencode-feishu-ws-bridge.mjs`

OpenCode auto-loads local plugins from this directory.
"#;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuBridgeInstallResult {
    pub opencode_config_dir: String,
    pub plugin_file: String,
    pub bridge_dir: String,
    pub start_command: String,
}

#[tauri::command]
pub async fn install_opencode_feishu_ws_bridge(
    state: tauri::State<'_, DbState>,
) -> Result<FeishuBridgeInstallResult, String> {
    let config_path = get_opencode_config_path(state).await?;
    let config_file_path = Path::new(&config_path);
    let config_dir = config_file_path
        .parent()
        .ok_or_else(|| "Failed to resolve OpenCode config directory".to_string())?;

    fs::create_dir_all(config_dir)
        .map_err(|e| format!("Failed to create OpenCode config directory: {}", e))?;

    let plugin_dir = config_dir.join("plugins");
    fs::create_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to create plugin directory: {}", e))?;

    let bridge_dir = config_dir.join("feishu-ws-bridge");
    fs::create_dir_all(&bridge_dir)
        .map_err(|e| format!("Failed to create bridge directory: {}", e))?;

    let plugin_file = plugin_dir.join(FEISHU_BRIDGE_PLUGIN_FILE);
    fs::write(&plugin_file, FEISHU_BRIDGE_PLUGIN_TEMPLATE.trim_start())
        .map_err(|e| format!("Failed to write plugin file: {}", e))?;

    fs::write(
        bridge_dir.join(FEISHU_BRIDGE_SERVER_FILE),
        FEISHU_BRIDGE_SERVER_TEMPLATE.trim_start(),
    )
    .map_err(|e| format!("Failed to write bridge server file: {}", e))?;

    fs::write(
        bridge_dir.join(FEISHU_BRIDGE_PKG_FILE),
        FEISHU_BRIDGE_PACKAGE_TEMPLATE.trim_start(),
    )
    .map_err(|e| format!("Failed to write package.json: {}", e))?;

    fs::write(
        bridge_dir.join(FEISHU_BRIDGE_ENV_EXAMPLE_FILE),
        FEISHU_BRIDGE_ENV_EXAMPLE_TEMPLATE.trim_start(),
    )
    .map_err(|e| format!("Failed to write .env.example: {}", e))?;

    fs::write(
        bridge_dir.join(FEISHU_BRIDGE_README_FILE),
        FEISHU_BRIDGE_README_TEMPLATE.trim_start(),
    )
    .map_err(|e| format!("Failed to write bridge README: {}", e))?;

    Ok(FeishuBridgeInstallResult {
        opencode_config_dir: config_dir.to_string_lossy().to_string(),
        plugin_file: plugin_file.to_string_lossy().to_string(),
        bridge_dir: bridge_dir.to_string_lossy().to_string(),
        start_command: "npm install && node --env-file=.env ./server.mjs".to_string(),
    })
}

/// Initialize default favorite plugins if database is empty
async fn init_default_favorite_plugins(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    for plugin_name in DEFAULT_FAVORITE_PLUGINS {
        let query = format!(
            "INSERT IGNORE INTO opencode_favorite_plugin {{ id: type::thing('opencode_favorite_plugin', $id), plugin_name: $plugin_name, created_at: $created_at }}"
        );
        db.query(&query)
            .bind(("id", *plugin_name))
            .bind(("plugin_name", *plugin_name))
            .bind(("created_at", now.clone()))
            .await
            .map_err(|e| format!("Failed to initialize favorite plugin: {}", e))?;
    }

    Ok(())
}

/// List all favorite plugins
/// Auto-initializes default plugins if database is empty
#[tauri::command]
pub async fn list_opencode_favorite_plugins(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<OpenCodeFavoritePlugin>, String> {
    let db = state.0.lock().await;

    // Check if there are any records
    let count_result: Result<Vec<Value>, _> = db
        .query("SELECT count() FROM opencode_favorite_plugin GROUP ALL")
        .await
        .map_err(|e| format!("Failed to count favorite plugins: {}", e))?
        .take(0);

    let is_empty = match count_result {
        Ok(records) => {
            records
                .first()
                .and_then(|r| r.get("count"))
                .and_then(|c| c.as_i64())
                .unwrap_or(0)
                == 0
        }
        Err(_) => true,
    };

    // Initialize default plugins if empty
    if is_empty {
        init_default_favorite_plugins(&db).await?;
    }

    // Query all favorite plugins ordered by created_at
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_plugin ORDER BY created_at ASC")
        .await
        .map_err(|e| format!("Failed to query favorite plugins: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            let plugins: Vec<OpenCodeFavoritePlugin> = records
                .into_iter()
                .map(adapter::from_db_value_favorite_plugin)
                .collect();
            Ok(plugins)
        }
        Err(e) => Err(format!("Failed to deserialize favorite plugins: {}", e)),
    }
}

/// Add a favorite plugin
/// Returns the created plugin, or existing one if already exists
#[tauri::command]
pub async fn add_opencode_favorite_plugin(
    state: tauri::State<'_, DbState>,
    plugin_name: String,
) -> Result<OpenCodeFavoritePlugin, String> {
    let db = state.0.lock().await;
    let now = chrono::Local::now().to_rfc3339();

    // Use INSERT IGNORE to avoid duplicates
    let query = "INSERT IGNORE INTO opencode_favorite_plugin { id: type::thing('opencode_favorite_plugin', $id), plugin_name: $plugin_name, created_at: $created_at }";
    db.query(query)
        .bind(("id", plugin_name.clone()))
        .bind(("plugin_name", plugin_name.clone()))
        .bind(("created_at", now.clone()))
        .await
        .map_err(|e| format!("Failed to add favorite plugin: {}", e))?;

    // Fetch the record (either newly created or existing)
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_plugin WHERE plugin_name = $plugin_name LIMIT 1")
        .bind(("plugin_name", plugin_name))
        .await
        .map_err(|e| format!("Failed to fetch favorite plugin: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.into_iter().next() {
                Ok(adapter::from_db_value_favorite_plugin(record))
            } else {
                Err("Failed to find favorite plugin after insert".to_string())
            }
        }
        Err(e) => Err(format!("Failed to deserialize favorite plugin: {}", e)),
    }
}

/// Delete a favorite plugin by plugin name
#[tauri::command]
pub async fn delete_opencode_favorite_plugin(
    state: tauri::State<'_, DbState>,
    plugin_name: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE FROM opencode_favorite_plugin WHERE plugin_name = $plugin_name")
        .bind(("plugin_name", plugin_name))
        .await
        .map_err(|e| format!("Failed to delete favorite plugin: {}", e))?;

    Ok(())
}

// ============================================================================
// Favorite Provider Commands
// ============================================================================

/// Sync providers from config file to database
/// Only inserts providers that don't exist in database
async fn sync_providers_from_config(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    config: &OpenCodeConfig,
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    if let Some(ref providers) = config.provider {
        for (provider_id, provider_config) in providers.iter() {
            // Extract npm and base_url from provider_config
            let npm = provider_config.npm.clone().unwrap_or_default();
            let base_url = provider_config
                .options
                .as_ref()
                .and_then(|o| o.base_url.clone())
                .unwrap_or_default();

            // Serialize provider_config to JSON
            let provider_config_json = serde_json::to_value(provider_config)
                .map_err(|e| format!("Failed to serialize provider config: {}", e))?;

            // Use INSERT IGNORE to only insert if not exists
            db.query("INSERT IGNORE INTO opencode_favorite_provider { id: type::thing('opencode_favorite_provider', $id), provider_id: $provider_id, npm: $npm, base_url: $base_url, provider_config: $provider_config, created_at: $created_at, updated_at: $updated_at }")
                .bind(("id", provider_id.clone()))
                .bind(("provider_id", provider_id.clone()))
                .bind(("npm", npm))
                .bind(("base_url", base_url))
                .bind(("provider_config", provider_config_json))
                .bind(("created_at", now.clone()))
                .bind(("updated_at", now.clone()))
                .await
                .map_err(|e| format!("Failed to sync favorite provider: {}", e))?;
        }
    }

    Ok(())
}

/// List all favorite providers
/// Auto-syncs providers from config file (inserts only if not exists in database)
#[tauri::command]
pub async fn list_opencode_favorite_providers(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<OpenCodeFavoriteProvider>, String> {
    // First, get config path BEFORE locking db to avoid deadlock
    let config_path_str = get_opencode_config_path(state.clone()).await?;
    let config_path = std::path::Path::new(&config_path_str);

    // Read and parse config file
    let config_opt = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .ok()
            .and_then(|content| json5::from_str::<OpenCodeConfig>(&content).ok())
    } else {
        None
    };

    // Now lock db and sync providers
    {
        let db = state.0.lock().await;

        if let Some(config) = config_opt {
            sync_providers_from_config(&db, &config).await?;
        }
    }

    // Query all favorite providers
    let db = state.0.lock().await;

    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_provider ORDER BY created_at ASC")
        .await
        .map_err(|e| format!("Failed to query favorite providers: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            let providers: Vec<OpenCodeFavoriteProvider> = records
                .into_iter()
                .filter_map(adapter::from_db_value_favorite_provider)
                .collect();
            Ok(providers)
        }
        Err(e) => Err(format!("Failed to deserialize favorite providers: {}", e)),
    }
}

/// Upsert (create or update) a favorite provider
/// Called automatically when user adds/modifies a provider
#[tauri::command]
pub async fn upsert_opencode_favorite_provider(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    provider_config: OpenCodeProvider,
    diagnostics: Option<OpenCodeDiagnosticsConfig>,
) -> Result<OpenCodeFavoriteProvider, String> {
    let db = state.0.lock().await;
    let now = chrono::Local::now().to_rfc3339();

    // Extract npm and base_url from provider_config
    let npm = provider_config.npm.clone().unwrap_or_default();
    let base_url = provider_config
        .options
        .as_ref()
        .and_then(|o| o.base_url.clone())
        .unwrap_or_default();

    // Serialize provider_config to JSON
    let provider_config_json = serde_json::to_value(&provider_config)
        .map_err(|e| format!("Failed to serialize provider config: {}", e))?;

    // Read existing record to preserve created_at and diagnostics if not provided
    let existing_record: Option<OpenCodeFavoriteProvider> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_provider WHERE provider_id = $provider_id LIMIT 1")
        .bind(("provider_id", provider_id.clone()))
        .await
        .map_err(|e| format!("Failed to query favorite provider: {}", e))?
        .take::<Vec<Value>>(0)
        .ok()
        .and_then(|records| records.into_iter().next())
        .and_then(adapter::from_db_value_favorite_provider);

    let has_existing = existing_record.is_some();
    let created_at = existing_record
        .as_ref()
        .map(|record| record.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let diagnostics_to_save = diagnostics.or_else(|| {
        existing_record
            .as_ref()
            .and_then(|record| record.diagnostics.clone())
    });

    if has_existing {
        db.query("UPDATE opencode_favorite_provider SET npm = $npm, base_url = $base_url, provider_config = $provider_config, diagnostics = $diagnostics, updated_at = $updated_at WHERE provider_id = $provider_id")
            .bind(("provider_id", provider_id.clone()))
            .bind(("npm", npm))
            .bind(("base_url", base_url))
            .bind(("provider_config", provider_config_json))
            .bind(("diagnostics", diagnostics_to_save))
            .bind(("updated_at", now.clone()))
            .await
            .map_err(|e| format!("Failed to update favorite provider: {}", e))?;
    } else {
        db.query("INSERT INTO opencode_favorite_provider { id: type::thing('opencode_favorite_provider', $id), provider_id: $provider_id, npm: $npm, base_url: $base_url, provider_config: $provider_config, diagnostics: $diagnostics, created_at: $created_at, updated_at: $updated_at }")
            .bind(("id", provider_id.clone()))
            .bind(("provider_id", provider_id.clone()))
            .bind(("npm", npm))
            .bind(("base_url", base_url))
            .bind(("provider_config", provider_config_json))
            .bind(("diagnostics", diagnostics_to_save))
            .bind(("created_at", created_at))
            .bind(("updated_at", now.clone()))
            .await
            .map_err(|e| format!("Failed to insert favorite provider: {}", e))?;
    }

    // Fetch and return the record
    let records_result: Result<Vec<Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM opencode_favorite_provider WHERE provider_id = $provider_id LIMIT 1")
        .bind(("provider_id", provider_id))
        .await
        .map_err(|e| format!("Failed to fetch favorite provider: {}", e))?
        .take(0);

    match records_result {
        Ok(records) => {
            if let Some(record) = records.into_iter().next() {
                adapter::from_db_value_favorite_provider(record)
                    .ok_or_else(|| "Failed to parse favorite provider".to_string())
            } else {
                Err("Failed to find favorite provider after upsert".to_string())
            }
        }
        Err(e) => Err(format!("Failed to deserialize favorite provider: {}", e)),
    }
}

/// Delete a favorite provider from database
#[tauri::command]
pub async fn delete_opencode_favorite_provider(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE FROM opencode_favorite_provider WHERE provider_id = $provider_id")
        .bind(("provider_id", provider_id))
        .await
        .map_err(|e| format!("Failed to delete favorite provider: {}", e))?;

    Ok(())
}
