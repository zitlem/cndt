// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shell feature through the JSON API (decision 0027): a plate hollowed
//! with one open face, with two, a wall too thick for the part, names that
//! survive edits, undo, and refusals with their reasons.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cndt_api::Api;
use serde_json::{Value, json};

struct Session {
    api: Api,
}

impl Session {
    fn new() -> Self {
        Self { api: Api::new() }
    }

    fn call(&mut self, request: Value) -> Value {
        serde_json::from_str(&self.api.handle_json(&request.to_string())).unwrap()
    }

    fn exec(&mut self, command: &str, args: Value) -> Value {
        let reply = self.call(json!({"method": "execute", "command": command, "args": args}));
        assert_eq!(reply["ok"], true, "{command} {args}: {reply}");
        reply["result"].clone()
    }

    fn refused(&mut self, command: &str, args: Value) -> String {
        let reply = self.call(json!({"method": "execute", "command": command, "args": args}));
        assert_eq!(reply["ok"], false, "{command} {args}: {reply}");
        reply["error"]["message"].as_str().unwrap().to_owned()
    }

    fn undo(&mut self) -> Value {
        let reply = self.call(json!({"method": "undo"}));
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"].clone()
    }

    fn state_now(&mut self) -> Value {
        self.call(json!({"method": "state"}))["result"].clone()
    }

    /// A 100 × 100 × 10 plate with its base on z = 0: feature 0.
    fn plate(&mut self) {
        self.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        self.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": -50, "y": -50}, "to": {"x": 50, "y": 50}}),
        );
        self.exec("feature.extrude", json!({"sketch": 0, "distance": 10}));
    }
}

fn volume(state: &Value) -> f64 {
    state["body"]["volume"].as_f64().unwrap()
}

fn face_names(state: &Value) -> Vec<String> {
    state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn status(state: &Value, feature: u64) -> Value {
    state["feature_status"][feature.to_string()].clone()
}

fn named(state: &Value, prefix: &str) -> Vec<String> {
    face_names(state)
        .into_iter()
        .filter(|n| n.starts_with(prefix))
        .collect()
}

const PLATE: f64 = 100.0 * 100.0 * 10.0;

#[test]
fn a_plate_shelled_open_at_the_top_keeps_the_closed_form_volume() {
    let mut s = Session::new();
    s.plate();
    let top = named(&s.state_now(), "f0/end/").remove(0);
    let state = s.exec("feature.shell", json!({"faces": [top], "thickness": 2}));
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    // The cavity is 96 × 96 × 8, open at the top.
    let expected = PLATE - 96.0 * 96.0 * 8.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{} against {expected}",
        volume(&state)
    );
    // Outer faces keep their names; each inner wall is named by the face
    // behind it; the open top stays as the rim.
    let names = face_names(&state);
    assert_eq!(
        names.iter().filter(|n| n.starts_with("f1/inner/")).count(),
        5,
        "{names:?}"
    );
    assert_eq!(names.len(), 11, "{names:?}");
    assert!(names.contains(&top_name(&state)), "{names:?}");
    // The command is shown by name, and undo takes the hollow away.
    let state = s.undo();
    assert!((volume(&state) - PLATE).abs() < 1e-3, "{}", volume(&state));
}

fn top_name(state: &Value) -> String {
    named(state, "f0/end/").remove(0)
}

#[test]
fn two_open_faces() {
    let mut s = Session::new();
    s.plate();
    let state = s.state_now();
    let top = named(&state, "f0/end/").remove(0);
    let side = named(&state, "f0/side/").remove(0);
    let state = s.exec(
        "feature.shell",
        json!({"faces": [top, side], "thickness": 2}),
    );
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    // The cavity runs out through the side: 98 × 96 × 8.
    let expected = PLATE - 98.0 * 96.0 * 8.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{} against {expected}",
        volume(&state)
    );
}

#[test]
fn a_wall_too_thick_for_the_part_fails_and_the_part_stands() {
    let mut s = Session::new();
    s.plate();
    let top = named(&s.state_now(), "f0/end/").remove(0);
    let state = s.exec("feature.shell", json!({"faces": [top], "thickness": 20}));
    let reason = status(&state, 1)["reason"].as_str().unwrap().to_owned();
    assert!(
        reason.contains("thicker") || reason.contains("wall"),
        "{reason}"
    );
    assert_eq!(status(&state, 0)["state"], "ok");
    assert!((volume(&state) - PLATE).abs() < 1e-3, "{}", volume(&state));
    // A thinner wall repairs it.
    let state = s.exec("feature.edit_shell", json!({"feature": 1, "thickness": 1}));
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    let expected = PLATE - 98.0 * 98.0 * 9.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{}",
        volume(&state)
    );
}

#[test]
fn names_survive_editing_the_part_before_it() {
    let mut s = Session::new();
    s.plate();
    let top = named(&s.state_now(), "f0/end/").remove(0);
    let state = s.exec("feature.shell", json!({"faces": [top], "thickness": 2}));
    let mut before = face_names(&state);
    before.sort();
    let state = s.exec(
        "feature.edit_extrude",
        json!({"feature": 0, "distance": 14}),
    );
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    let mut after = face_names(&state);
    after.sort();
    assert_eq!(before, after);
    let expected = 100.0 * 100.0 * 14.0 - 96.0 * 96.0 * 12.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{}",
        volume(&state)
    );
}

#[test]
fn a_shell_after_a_chamfer_keeps_the_chamfer_face() {
    let mut s = Session::new();
    s.plate();
    let state = s.state_now();
    let top = named(&state, "f0/end/").remove(0);
    let bottom = named(&state, "f0/start/").remove(0);
    s.exec(
        "feature.chamfer",
        json!({"faces": [bottom], "distance": 0.5}),
    );
    let state = s.exec("feature.shell", json!({"faces": [top], "thickness": 0.3}));
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    assert!(volume(&state) < PLATE - 1.0);
    assert!(
        !named(&state, "f1/blend/").is_empty(),
        "the chamfer faces keep their names: {:?}",
        face_names(&state)
    );
}

#[test]
fn bad_shells_say_what_is_wrong() {
    let mut s = Session::new();
    s.plate();
    let state = s.state_now();
    let top = named(&state, "f0/end/").remove(0);
    let message = s.refused("feature.shell", json!({"thickness": 2}));
    assert!(message.contains("faces"), "{message}");
    let message = s.refused("feature.shell", json!({"faces": [], "thickness": 2}));
    assert!(message.contains("faces"), "{message}");
    let message = s.refused("feature.shell", json!({"faces": [top.clone()]}));
    assert!(message.contains("thickness"), "{message}");
    let message = s.refused(
        "feature.shell",
        json!({"faces": [top.clone()], "thickness": 0}),
    );
    assert!(message.contains("more than zero"), "{message}");
    let message = s.refused("feature.edit_shell", json!({"feature": 0, "thickness": 1}));
    assert!(message.contains("not a shell"), "{message}");
    // A face that does not exist fails the feature with the reason.
    let state = s.exec(
        "feature.shell",
        json!({"faces": ["f9/end/1"], "thickness": 2}),
    );
    let reason = status(&state, 1)["reason"].as_str().unwrap().to_owned();
    assert!(reason.contains("no longer exists"), "{reason}");
    assert_eq!(status(&state, 0)["state"], "ok");
    // Every face open leaves nothing.
    let mut all = face_names(&s.state_now());
    all.sort();
    all.dedup();
    s.exec("feature.delete", json!({"feature": 1}));
    let state = s.exec("feature.shell", json!({"faces": all, "thickness": 1}));
    let reason = status(&state, 2)["reason"].as_str().unwrap().to_owned();
    assert!(reason.contains("stay closed"), "{reason}");
}
