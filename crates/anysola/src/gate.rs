//! The minimum-version gate: released builds older than the gateway's `minimum` refuse to start.
//! Offline or a slow gateway lets the command run.

use std::time::Duration;

use anysola_core::gateway::Gateway;
use anysola_core::version::{VERSION, compare, is_dev_build};

/// The message to print (and exit 1) when this build is no longer supported.
pub async fn minimum_version() -> Option<String> {
    if is_dev_build(VERSION) {
        return None;
    }
    let info = Gateway::new(None).version(Duration::from_secs(3)).await?;
    let minimum = info.minimum?;
    compare(VERSION, &minimum).is_lt().then(|| {
        format!("Anysola Code {VERSION} is no longer supported (minimum {minimum}).\nUpdate Anysola Code with: anysola update\n")
    })
}
