//! Anysola Code core: config, storage, the gateway client, the agent loop and tools. Front ends
//! (`run`, the TUI, ACP) drive it through [`api`]; nothing here knows about terminals.

pub mod ids;
pub mod json;
pub mod paths;
pub mod permission;
pub mod project;
pub mod text;
pub mod time;
pub mod tool;
