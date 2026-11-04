//! Account commands: `login`, `logout`, `status`, `usage`, `devices`.

use crate::cli::{DevicesCommand, UsageArgs};

pub async fn login(_force: bool) -> i32 {
    crate::fail("not implemented")
}

pub async fn logout() -> i32 {
    crate::fail("not implemented")
}

pub async fn status() -> i32 {
    crate::fail("not implemented")
}

pub async fn usage(_args: UsageArgs) -> i32 {
    crate::fail("not implemented")
}

pub async fn devices(_command: Option<DevicesCommand>) -> i32 {
    crate::fail("not implemented")
}
