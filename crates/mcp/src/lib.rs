//! The MCP client: stdio and Streamable HTTP servers from config, their tools in Code Mode (or as
//! direct tools), resources, prompts, elicitation forms and OAuth sign-in.
//!
//! Front ends call [`Runtime::attach`] once their [`anysola_core::api::Core`] is up.

mod client;
mod elicit;
mod http;
pub mod oauth;
mod rpc;
mod runtime;
mod stdio;
mod tool;

pub use runtime::Runtime;
