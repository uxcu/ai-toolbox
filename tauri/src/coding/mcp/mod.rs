//! MCP Server Management Module
//!
//! This module provides MCP (Model Context Protocol) server management functionality.
//! It allows users to configure and sync MCP servers across multiple AI coding tools.

pub mod adapter;
pub mod command_normalize;
pub mod commands;
pub mod config_sync;
pub mod format_configs;
pub mod mcp_store;
pub mod opencode_path;
pub mod tray_support;
pub mod types;

pub use commands::*;
