//! Shared Tools Module
//!
//! This module provides unified tool adapter functionality for both Skills and MCP features.
//! It contains built-in tool configurations, custom tool management, and detection logic.

pub mod builtin;
pub mod custom_store;
pub mod detection;
pub mod path_utils;
pub mod types;

pub use builtin::*;
pub use detection::*;
pub use path_utils::*;
pub use types::*;
