// SPDX-License-Identifier: AGPL-3.0-or-later
//! Crash recovery snapshots (decision 0026). A "crash" is dropping the `Api`
//! without saving: nothing runs on drop, so the snapshot stays behind.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use cndt_api::Api;
use serde_json::{Value, json};

fn call(api: &mut Api, request: &Value) -> Value {
    serde_json::from_str(&api.handle_json(&request.to_string())).unwrap()
}

fn ok(api: &mut Api, request: &Value) -> Value {
    let reply = call(api, request);
    assert_eq!(reply["ok"], true, "{request}: {reply}");
    reply["result"].clone()
}

fn project_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cndt-recovery-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn saved_project(dir: &Path) -> (Api, PathBuf) {
    let path = dir.join("part.cndt");
    let mut api = Api::new();
    ok(&mut api, &json!({"method": "new_document", "name": "Part"}));
    ok(
        &mut api,
        &json!({"method": "save_as", "path": path.display().to_string()}),
    );
    (api, path)
}

fn rename(api: &mut Api, name: &str) {
    ok(
        api,
        &json!({"method": "execute", "command": "document.rename", "args": {"name": name}}),
    );
}

fn available(api: &mut Api, path: &Path) -> bool {
    let reply = ok(
        api,
        &json!({"method": "recovery", "path": path.display().to_string()}),
    );
    reply["available"].as_bool().unwrap()
}

#[test]
fn a_crash_after_an_edit_leaves_a_snapshot_that_restores_the_edit() {
    let dir = project_dir("restore");
    let (mut api, path) = saved_project(&dir);
    rename(&mut api, "Edited");
    ok(&mut api, &json!({"method": "snapshot"}));
    assert!(available(&mut api, &path));
    drop(api); // the crash: no save

    let mut next = Api::new();
    ok(
        &mut next,
        &json!({"method": "open", "path": path.display().to_string()}),
    );
    assert!(available(&mut next, &path), "offered at the next open");
    // The project file still holds the last saved state.
    let state = ok(&mut next, &json!({"method": "state"}));
    assert_eq!(state["document"]["name"], "Part");

    let state = ok(
        &mut next,
        &json!({"method": "open_recovery", "path": path.display().to_string()}),
    );
    assert_eq!(state["document"]["name"], "Edited");
    assert_eq!(state["dirty"], true);
    assert_eq!(state["path"], path.display().to_string());

    // Saving makes the recovered state the project and removes the snapshot.
    ok(&mut next, &json!({"method": "save"}));
    assert!(!available(&mut next, &path));
    let mut third = Api::new();
    let state = ok(
        &mut third,
        &json!({"method": "open", "path": path.display().to_string()}),
    );
    assert_eq!(state["document"]["name"], "Edited");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn nothing_is_written_for_a_saved_or_a_fileless_document() {
    let dir = project_dir("none");
    let (mut api, path) = saved_project(&dir);
    ok(&mut api, &json!({"method": "snapshot"}));
    assert!(!available(&mut api, &path), "a saved document needs none");

    let mut fresh = Api::new();
    ok(
        &mut fresh,
        &json!({"method": "new_document", "name": "New"}),
    );
    rename(&mut fresh, "Changed");
    ok(&mut fresh, &json!({"method": "snapshot"}));
    assert!(!available(&mut fresh, &path));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_deliberate_discard_removes_the_snapshot() {
    let dir = project_dir("discard");
    let (mut api, path) = saved_project(&dir);
    rename(&mut api, "Edited");
    ok(&mut api, &json!({"method": "snapshot"}));
    assert!(available(&mut api, &path));
    // Opening the project again and discarding the changes is a decision.
    ok(
        &mut api,
        &json!({"method": "open", "path": path.display().to_string(), "discard_changes": true}),
    );
    assert!(!available(&mut api, &path));

    rename(&mut api, "Again");
    ok(&mut api, &json!({"method": "snapshot"}));
    assert!(available(&mut api, &path));
    ok(
        &mut api,
        &json!({"method": "discard_recovery", "path": path.display().to_string()}),
    );
    assert!(!available(&mut api, &path));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recovery_refusals_explain_themselves() {
    let dir = project_dir("refuse");
    let (mut api, path) = saved_project(&dir);
    let reply = call(
        &mut api,
        &json!({"method": "open_recovery", "path": path.display().to_string()}),
    );
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["kind"], "no_recovery");

    // A snapshot that is not a project is refused, not a panic.
    std::fs::write(dir.join("part.cndt.recovery"), b"not a project").unwrap();
    let reply = call(
        &mut api,
        &json!({"method": "open_recovery", "path": path.display().to_string()}),
    );
    assert_eq!(reply["ok"], false);

    // Unsaved changes are guarded like any open.
    rename(&mut api, "Dirty");
    std::fs::remove_file(dir.join("part.cndt.recovery")).unwrap();
    ok(&mut api, &json!({"method": "snapshot"}));
    let reply = call(
        &mut api,
        &json!({"method": "open_recovery", "path": path.display().to_string()}),
    );
    assert_eq!(reply["error"]["kind"], "unsaved_changes");
    std::fs::remove_dir_all(dir).unwrap();
}
