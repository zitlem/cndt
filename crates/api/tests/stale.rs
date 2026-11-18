// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stale operation markers (architecture rule 9): an operation whose
//! toolpath was planned shows as stale once the design it was planned from
//! changes, and not when something irrelevant changes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cndt_api::Api;
use serde_json::{Value, json};

struct Session {
    api: Api,
}

impl Session {
    fn call(&mut self, request: Value) -> Value {
        serde_json::from_str(&self.api.handle_json(&request.to_string())).unwrap()
    }

    fn exec(&mut self, command: &str, args: Value) -> Value {
        let reply = self.call(json!({"method": "execute", "command": command, "args": args}));
        assert_eq!(reply["ok"], true, "{command} {args}: {reply}");
        reply["result"].clone()
    }

    fn stale(&mut self) -> Vec<u64> {
        let state = self.call(json!({"method": "state"}));
        state["result"]["stale_operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_u64().unwrap())
            .collect()
    }
}

/// A 60 × 40 × 30 block with one top setup that faces it.
fn block() -> Session {
    let mut s = Session { api: Api::new() };
    s.exec("sketch.create", json!({"plane": "xy", "name": "Outline"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 30}));
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    s.exec("cam.add_setup", json!({"side": "top"}));
    s.exec("cam.add_operation", json!({"setup": 0, "type": "face"}));
    s
}

#[test]
fn nothing_is_marked_before_a_toolpath_was_planned() {
    let mut s = block();
    assert_eq!(s.stale(), Vec::<u64>::new());
    s.exec(
        "feature.edit_extrude",
        json!({"feature": 0, "distance": 35}),
    );
    assert_eq!(s.stale(), Vec::<u64>::new());
}

#[test]
fn a_design_edit_marks_the_operation_until_it_is_planned_again() {
    let mut s = block();
    s.api.toolpath_preview(Some(0)).unwrap();
    assert_eq!(s.stale(), Vec::<u64>::new());

    s.exec(
        "feature.edit_extrude",
        json!({"feature": 0, "distance": 35}),
    );
    assert_eq!(s.stale(), vec![0]);

    // Planned again from the new design: fresh.
    s.api.toolpath_preview(Some(0)).unwrap();
    assert_eq!(s.stale(), Vec::<u64>::new());
}

#[test]
fn undo_back_to_the_planned_design_clears_the_marker() {
    let mut s = block();
    s.api.toolpath_preview(Some(0)).unwrap();
    s.exec("cam.set_stock", json!({"side": 5, "top": 2, "bottom": 3}));
    assert_eq!(s.stale(), vec![0]);
    s.call(json!({"method": "undo"}));
    assert_eq!(s.stale(), Vec::<u64>::new());
}

#[test]
fn an_irrelevant_edit_does_not_mark_it() {
    let mut s = block();
    s.api.toolpath_preview(Some(0)).unwrap();
    // A new name, a parameter nothing uses, and another operation joining
    // the setup do not change this operation's toolpath.
    s.exec("document.rename", json!({"name": "Renamed"}));
    s.exec(
        "parameter.set",
        json!({"name": "unused", "value": 3, "unit": "mm"}),
    );
    s.exec("cam.add_operation", json!({"setup": 0, "type": "holes"}));
    assert_eq!(s.stale(), Vec::<u64>::new());
}

#[test]
fn only_the_operations_of_a_changed_setup_are_marked() {
    let mut s = block();
    s.exec("cam.add_setup", json!({"side": "top", "work_offset": 2}));
    s.exec("cam.add_operation", json!({"setup": 1, "type": "face"}));
    s.api.toolpath_preview(Some(0)).unwrap();
    s.api.toolpath_preview(Some(1)).unwrap();
    assert_eq!(s.stale(), Vec::<u64>::new());
    // A vise belongs to the second setup alone.
    s.exec("cam.set_vise", json!({"setup": 1, "grip": 10}));
    assert_eq!(s.stale(), vec![1]);
}

#[test]
fn a_new_document_forgets_what_was_planned() {
    let mut s = block();
    s.api.toolpath_preview(Some(0)).unwrap();
    s.call(json!({"method": "new_document", "name": "Other", "discard_changes": true}));
    assert_eq!(s.stale(), Vec::<u64>::new());
}
