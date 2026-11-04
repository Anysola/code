//! Test support: an in-process fake Anysola gateway with scripted SSE answers, and isolated homes
//! for running the real `anysola` binary against it.

mod gateway;
mod home;
pub mod sse;

pub use gateway::{
    DEVICE_TOKEN, DevicePoll, FakeGateway, Recorded, Reply, Script, default_tokens, default_usage,
};
pub use home::{Output, TestHome};
