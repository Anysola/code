//! Updates: the version Admin marks latest (`GET /v1/code/version`, as `anysola update` reads
//! it), installed from that version's GitHub release, whose `latest-desktop.json` is signed with
//! the key in tauri.conf.json. A version without that release is never offered.

use std::time::Duration;

use anysola_core::gateway::Gateway;
use anysola_core::version::compare;
use serde_json::json;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

use crate::{Answer, text};

const RELEASES: &str = "https://github.com/Anysola/code/releases";

/// The update found by the last check, kept for `update_install`.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[tauri::command]
pub async fn update_check(handle: AppHandle, pending: State<'_, Pending>) -> Answer {
    // Development builds carry the placeholder version: nothing to offer them.
    if cfg!(debug_assertions) {
        return Ok(json!({"available": false}));
    }
    let current = handle.package_info().version.to_string();
    let Some(info) = Gateway::new(None).version(Duration::from_secs(8)).await else {
        return Ok(json!({"available": false}));
    };
    if compare(&info.latest, &current).is_le() {
        return Ok(json!({"available": false}));
    }
    // A .deb or a build run from the tree cannot replace itself; an AppImage can.
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        return Ok(json!({
            "available": true,
            "version": info.latest,
            "manual": format!("{RELEASES}/tag/v{}", info.latest),
        }));
    }
    let manifest = format!("{RELEASES}/download/v{}/latest-desktop.json", info.latest);
    let update = handle
        .updater_builder()
        .endpoints(vec![manifest.parse().map_err(text)?])
        .map_err(text)?
        .build()
        .map_err(text)?
        .check()
        .await
        .map_err(text)?;
    let Some(update) = update else {
        return Ok(json!({"available": false}));
    };
    let version = update.version.clone();
    *pending.0.lock().await = Some(update);
    Ok(json!({"available": true, "version": version, "notes": info.notes}))
}

/// Downloads and installs the checked update, then starts the new version.
#[tauri::command]
pub async fn update_install(handle: AppHandle, pending: State<'_, Pending>) -> Answer {
    let update = pending
        .0
        .lock()
        .await
        .take()
        .ok_or("No update is ready; check again.")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(text)?;
    handle.restart()
}
