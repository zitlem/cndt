// SPDX-License-Identifier: AGPL-3.0-or-later
//! The CNDT desktop app: a Tauri window around the CNDT API.
//!
//! The frontend (in `ui/`) reaches the core through exactly five commands,
//! and nothing else (architecture rules 5 and 12):
//!
//! - `api`: one JSON request in, one JSON response out, exactly as
//!   `cndt_api::Api::handle_json` and `cndt api` on the command line.
//! - `step_preview`: a STEP file's display mesh as raw bytes, in the layout
//!   documented on `cndt_api::Api::step_preview`.
//! - `body_mesh`: the rebuilt part's display mesh, in the same layout.
//! - `body_edges`: the rebuilt part's edges as polylines, in the layout
//!   documented on `cndt_api::EDGES_MAGIC`.
//! - `toolpath_preview`: a setup's toolpath for the 3D view, in the layout
//!   documented on `cndt_api::Api::toolpath_preview`.
//! - `simulation_mesh`: a setup's simulated stock, as
//!   `cndt_api::Api::simulation_mesh` describes.
//! - `reference_mesh`: a reference mesh's triangles (decision 0018).
//! - `posted_preview`: a setup's posted G-code read back and drawn, in the
//!   `toolpath_preview` layout.
//!
//! - `job_cancel` and `job_progress`: stop the running simulation or
//!   verification and read how far it is, without waiting for the document
//!   lock the running request holds (decision 0035).
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
struct AppState {
    api: Mutex<Api>,
    /// Cancel and progress of the running simulation or verification, kept
    /// outside the lock the running request holds (decision 0035).
    jobs: cndt_api::JobControl,
}

impl AppState {
    fn api(&self) -> MutexGuard<'_, Api> {
        // A poisoned lock means a command panicked mid-request. The document
        // itself is only changed through commands that either finish or
        // change nothing, so it is still consistent.
        self.api.lock().unwrap_or_else(PoisonError::into_inner)
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

/// Asks the running simulation or verification to stop. It does not wait for
/// the document lock, which that request holds.
#[tauri::command]
fn job_cancel(state: State<'_, AppState>) {
    state.jobs.cancel();
}

/// How far the running simulation has got: steps done and steps in all.
#[tauri::command]
fn job_progress(state: State<'_, AppState>) -> (usize, usize) {
    state.jobs.progress()
}

#[tauri::command(async)]
fn step_preview(path: String, state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .step_preview(Path::new(&path))
        .map(Response::new)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
fn body_mesh(state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .body_mesh()
        .map(Response::new)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
fn body_edges(state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .body_edges()
        .map(Response::new)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
fn toolpath_preview(setup: Option<u32>, state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .toolpath_preview(setup)
        .map(Response::new)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
fn simulation_mesh(setup: Option<u32>, state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .simulation_mesh(setup)
        .map(Response::new)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
fn posted_preview(
    setup: Option<u32>,
    post: String,
    definition: Option<String>,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    state
        .api()
        .posted_preview(setup, &post, definition)
        .map(Response::new)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
fn reference_mesh(feature: u32, state: State<'_, AppState>) -> Result<Response, String> {
    state
        .api()
        .reference_mesh(feature)
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

fn new_state() -> AppState {
    let api = Api::new();
    let jobs = api.job_control();
    AppState {
        api: Mutex::new(api),
        jobs,
    }
}

fn main() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(new_state())
        .invoke_handler(tauri::generate_handler![
            api,
            job_cancel,
            job_progress,
            step_preview,
            body_mesh,
            body_edges,
            toolpath_preview,
            simulation_mesh,
            reference_mesh,
            posted_preview
        ])
        .run(context());
    if let Err(error) = result {
        eprintln!("CNDT could not start: {error}");
        std::process::exit(1);
    }
}
