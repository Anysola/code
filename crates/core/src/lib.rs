//! Anysola Code core: config, storage, the gateway client, the agent loop and tools. Front ends
//! (`run`, the TUI, ACP) drive it through [`api`]; nothing here knows about terminals.

pub mod agent;
pub mod api;
pub mod codemode;
pub mod config;
pub mod credits;
pub mod events;
pub mod gateway;
pub mod ids;
pub mod instructions;
pub mod json;
pub mod log;
pub mod markdown;
pub mod model;
pub mod paths;
pub mod permission;
pub mod project;
pub mod search;
pub mod session;
pub mod slug;
pub mod storage;
pub mod text;
pub mod time;
pub mod tool;
pub mod version;
pub mod worktree;
