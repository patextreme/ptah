//! ptah: Luau-scripted multi-agent orchestration over the Agent Client
//! Protocol.
//!
//! This crate is the composition root *and* the permanent `ptah` facade:
//! a flat re-export of the workspace member crates, so existing imports
//! (`ptah::acp`, `ptah::script`, `ptah::render`, `ptah::config`,
//! `ptah::task`, …) keep resolving unchanged. Adapter selection (the
//! ACP stdio transport behind `AgentTransport`) is composed here in
//! `cli`, the only crate allowed to see every member.
//!
//! Facade rules (change ② design D3): flat `pub use` list, no glob
//! re-exports, no logic. The member crates are private workspace
//! members; this surface is the package's public API.

pub use ptah_acp as acp;
pub use ptah_check as check;
pub use ptah_config as config_fs;
pub use ptah_luau as script;
pub use ptah_render as render;
pub use ptah_result as result_wire;

/// Compat re-export: the config model lives in `ptah-core`.
pub use ptah_core::config;

/// Compat re-export: task semantics live in `ptah-core`.
pub use ptah_core::task;

/// Compat re-export: the version string lives in `ptah-core`.
pub use ptah_core::VERSION;

pub mod ask;
pub mod bridge;
pub mod cli;
pub mod exec;
