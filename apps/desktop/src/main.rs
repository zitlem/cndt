// SPDX-License-Identifier: AGPL-3.0-or-later
//! The CNDT desktop app: a Tauri window around the CNDT API.
//!
//! The frontend (in `ui/`) reaches the core through exactly two commands,
//! and nothing else (architecture rules 5 and 12):
//!
//! - `api`: one JSON request in, one JSON response out, exactly as
//!   `cndt_api::Api::handle_json` and `cndt api` on the command line.
//! - `step_preview`: a STEP file's display mesh as raw bytes, in the layout
//!   documented on `cndt_api::Api::step_preview`.
//!
//! Both run off the main thread, so a slow import never freezes the window.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use cndt_api::Api;
use tauri::State;
use tauri::ipc::Response;

/// The one open document. The API is not thread-safe, so every command
/// takes the lock.
struct AppState(Mutex<Api>);

impl AppState {
    fn api(&self) -> MutexGuard<'_, Api> {
        // A poisoned lock means a command panicked mid-request. The document
        // itself is only changed through commands that either finish or
        // change nothing, so it is still consistent.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[tauri::command(async)]
fn api(request: String, state: State<'_, AppState>) -> String {
    let response = state.api().handle_json(&request);
    // Debug builds trace the API traffic, to check the frontend end to end.
    if cfg!(debug_assertions) {
        eprintln!("api> {request}");
        eprintln!("api< {}", response.chars().take(200).collect::<String>());
    }
    response
}

#[tauri::command(async)]
fn step_preview(path: String, state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .step_preview(Path::new(&path))
        .map(Response::new)
        .map_err(|error| error.message)
}

/// Tauri's generated startup context uses `HashMap` internally. It is
/// configuration loading, not anything that reaches a toolpath or a file, so
/// the determinism rule (architecture rule 11) does not apply to it.
#[allow(clippy::disallowed_types)]
fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

fn main() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState(Mutex::new(Api::new())))
        .invoke_handler(tauri::generate_handler![api, step_preview])
        .run(context());
    if let Err(error) = result {
        eprintln!("CNDT could not start: {error}");
        std::process::exit(1);
    }
}
