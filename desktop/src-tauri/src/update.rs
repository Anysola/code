//! Updates: the version Admin marks latest (`GET /v1/code/version`), installed from that version's GitHub release, whose `latest-desktop.json` is signed with
//! the key in tauri.conf.json. A version without that release is never offered.

use std::time::Duration;

use anysola_core::gateway::Gateway;
use anysola_core::version::compare;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

use crate::{Answer, App, EXIT_GRACE, text};

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
    // So do branch builds from CI, which no release would be newer than in any useful sense.
    if current.starts_with("0.") {
        return Ok(json!({"available": false}));
    }
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
    let exiting = handle.clone();
    let update = handle
        .updater_builder()
        // On Windows the installer starts and the process exits on the spot, with no
        // `RunEvent::Exit`: what that event stops is stopped here.
        .on_before_exit(move || before_exit(&exiting))
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
    // Stays pending until it is installed: a download that fails can be tried again.
    let update = pending
        .0
        .lock()
        .await
        .clone()
        .ok_or("No update is ready; check again.")?;
    let progress = handle.clone();
    let mut downloaded: u64 = 0;
    let mut told: Option<std::time::Instant> = None;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                // A few times a second is enough for a progress bar.
                let due = told.is_none_or(|at| at.elapsed() >= Duration::from_millis(150));
                let done = total.is_some_and(|t| downloaded >= t);
                if due || done {
                    told = Some(std::time::Instant::now());
                    let _ = progress.emit(
                        "core",
                        json!({"type": "update", "downloaded": downloaded, "total": total}),
                    );
                }
            },
            || {},
        )
        .await
        .map_err(text)?;
    handle.restart()
}

/// The cleanup of `RunEvent::Exit` in `main`, for the Windows installer's exit. It runs inside
/// the install command on an async worker, where blocking on the runtime would panic: a thread
/// of its own does it, and is waited for no longer than its two graces and a little more. (With
/// every worker of the runtime busy, the timer its timeout needs might never fire.)
fn before_exit(handle: &AppHandle) {
    let worker = handle.clone();
    bounded(EXIT_GRACE * 2 + Duration::from_secs(1), move || {
        let app = worker.state::<App>();
        tauri::async_runtime::block_on(async {
            let _ = tokio::time::timeout(EXIT_GRACE, app.drop_cores()).await;
        });
        if let Some(discord) = app.discord.get() {
            discord.stop(EXIT_GRACE);
        }
    });
    // What the plugin's own hook does, which this one replaces.
    handle.cleanup_before_exit();
}

/// Runs `work` on a thread of its own and waits for it at most `limit`; past that it is left to
/// end with the process. Returns whether it finished.
fn bounded(limit: Duration, work: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    finished.recv_timeout(limit).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleanup_that_hangs_is_given_up() {
        let started = std::time::Instant::now();
        assert!(!bounded(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(30))
        }));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(bounded(Duration::from_secs(5), || {}));
    }
}
