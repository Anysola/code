//! Test support: an in-process fake Anysola gateway with scripted SSE answers, and isolated homes
//! for running the real `anysola` binary against it.

mod gateway;
mod home;
pub mod sse;

pub use gateway::{FakeGateway, Recorded, Reply, Script, default_config};
pub use home::{Output, TestHome};
