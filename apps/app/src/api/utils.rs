use serde::{Deserialize, Serialize};
use tauri::Runtime;
use tauri_plugin_opener::OpenerExt;
use theseus::{
    handler,
    prelude::{CommandPayload, DirectoryInfo},
};

use crate::api::{Result, TheseusSerializableError};
use dashmap::DashMap;
use std::path::{Path, PathBuf};
use theseus::prelude::canonicalize;
use url::Url;

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("utils")
        .invoke_handler(tauri::generate_handler![
            get_os,
            should_disable_mouseover,
            highlight_in_folder,
            open_path,
            show_launcher_logs_folder,
            progress_bars_list,
            get_opening_command
        ])
        .build()
}

/// Gets OS
#[tauri::command]
pub fn get_os() -> OS {
    #[cfg(target_os = "windows")]
    let os = OS::Windows;
    #[cfg(target_os = "linux")]
    let os = OS::Linux;
    #[cfg(target_os = "macos")]
    let os = OS::MacOS;
    os
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum OS {
    Windows,
    Linux,
    MacOS,
}

// Lists active progress bars
// Create a new HashMap with the same keys
// Values provided should not be used directly, as they are not guaranteed to be up-to-date
#[tauri::command]
pub async fn progress_bars_list()
-> Result<DashMap<uuid::Uuid, theseus::LoadingBar>> {
    let res = theseus::EventState::list_progress_bars().await?;
    Ok(res)
}

// disables mouseover and fixes a random crash error only fixed by recent versions of macos
#[tauri::command]
pub async fn should_disable_mouseover() -> bool {
    if cfg!(target_os = "macos") {
        // We try to match version to 12.2 or higher. If unrecognizable to pattern or lower, we default to the css with disabled mouseover for safety
        if let tauri_plugin_os::Version::Semantic(major, minor, _) =
            tauri_plugin_os::version()
        {
            if major >= 12 && minor >= 3 {
                // Mac os version is 12.3 or higher, we allow mouseover
                return false;
            }
        }
        true
    } else {
        // Not macos, we allow mouseover
        false
    }
}

#[tauri::command]
pub fn highlight_in_folder<R: Runtime>(
    app: tauri::AppHandle<R>,
    path: PathBuf,
) {
    if let Err(e) = app.opener().reveal_item_in_dir(path) {
        tracing::error!("Failed to highlight file in folder: {}", e);
    }
}

#[tauri::command]
pub fn open_path<R: Runtime>(app: tauri::AppHandle<R>, path: PathBuf) {
    if let Err(e) = app.opener().open_path(path.to_string_lossy(), None::<&str>)
    {
        tracing::error!("Failed to open path: {}", e);
    }
}

#[tauri::command]
pub fn show_launcher_logs_folder<R: Runtime>(app: tauri::AppHandle<R>) {
    let path = DirectoryInfo::launcher_logs_dir().unwrap_or_default();
    // failure to get folder just opens filesystem
    // (ie: if in debug mode only and launcher_logs never created)
    open_path(app, path);
}

// Get opening command
// For example, if a user clicks on an .mrpack to open the app.
// This should be called once and only when the app is done booting up and ready to receive a command
// Returns a Command struct- see events.js
#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn get_opening_command(
    state: tauri::State<'_, crate::macos::deep_link::InitialPayload>,
) -> Result<Option<CommandPayload>> {
    let payload = state.payload.lock().await;

    return if let Some(payload) = payload.as_ref() {
        tracing::info!("opening command {payload}");

        Ok(Some(handler::parse_command(payload).await?))
    } else {
        Ok(None)
    };
}

#[tauri::command]
#[cfg(not(target_os = "macos"))]
pub async fn get_opening_command() -> Result<Option<CommandPayload>> {
    // Tauri is not CLI, we use arguments as path to file to call
    let cmd_arg = std::env::args_os().nth(1);

    tracing::info!("opening command {cmd_arg:?}");

    let cmd_arg = cmd_arg.map(|path| path.to_string_lossy().to_string());
    if let Some(cmd) = cmd_arg {
        tracing::debug!("Opening command: {:?}", cmd);
        return Ok(Some(handler::parse_command(&cmd).await?));
    }
    Ok(None)
}

// helper function called when redirected by a weblink (ie: modrith://do-something) or when redirected by a .mrpack file (in which case its a filepath)
// We hijack the deep link library (which also contains functionality for instance-checking)
pub async fn handle_command(command: String) -> Result<()> {
    tracing::info!("handle command: {command}");
    Ok(theseus::handler::parse_and_emit_command(&command).await?)
}

// Remove when (and if) https://github.com/tauri-apps/tauri/issues/12022 is implemented
pub(crate) fn tauri_convert_file_src(path: &Path) -> Result<Url> {
    #[cfg(any(windows, target_os = "android"))]
    const BASE: &str = "http://asset.localhost/";
    #[cfg(not(any(windows, target_os = "android")))]
    const BASE: &str = "asset://localhost/";

    macro_rules! theseus_try {
        ($test:expr) => {
            match $test {
                Ok(val) => val,
                Err(e) => {
                    return Err(TheseusSerializableError::Theseus(e.into()))
                }
            }
        };
    }

    let path = theseus_try!(canonicalize(path));
    let path = path.to_string_lossy();
    let encoded = urlencoding::encode(&path);

    Ok(theseus_try!(Url::parse(&format!("{BASE}{encoded}"))))
}
