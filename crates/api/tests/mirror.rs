// SPDX-License-Identifier: AGPL-3.0-or-later
//! The mirror feature through the JSON API (decision 0023): a mirrored hole, a
//! mirrored extrude that doubles the part, a mirrored revolve, names that
//! survive editing the original, undo, and refusals with their reasons.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cndt_api::Api;
use serde_json::{Value, json};
use std::f64::consts::PI;

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

    fn point(&mut self, sketch: u64, x: f64, y: f64) -> u64 {
        let state = self.exec(
            "sketch.add_point",
            json!({"sketch": sketch, "at": {"x": x, "y": y}}),
        );
        state["document"]["sketches"][sketch.to_string()]["sketch"]["next_entity"]
            .as_u64()
            .unwrap()
            - 1
    }

    /// A 100 × 100 × 10 plate centred on the origin: feature 0.
    fn plate(&mut self) {
        self.plate_of(10.0);
    }

    /// The same plate, extruded `distance` along the sketch's normal.
    fn plate_of(&mut self, distance: f64) {
        self.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        self.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": -50, "y": -50}, "to": {"x": 50, "y": 50}}),
        );
        self.exec(
            "feature.extrude",
            json!({"sketch": 0, "distance": distance}),
        );
    }

    /// A new sketch on the plate's top face.
    fn on_top(&mut self) -> u64 {
        let state = self.exec("sketch.create", json!({"plane": "xy"}));
        let top = face_names(&state)
            .into_iter()
            .find(|n| n.starts_with("f0/end/"))
            .unwrap();
        let state = self.exec("sketch.create", json!({"face": top}));
        state["document"]["next_sketch"].as_u64().unwrap() - 1
    }

    /// A 150 × 100 × 10 plate from x = -100 to 50: feature 0, with one hole
    /// of diameter 6 at (30, 0) through its top: feature 1.
    fn lopsided_plate_with_hole(&mut self) {
        self.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        self.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": -100, "y": -50}, "to": {"x": 50, "y": 50}}),
        );
        self.exec("feature.extrude", json!({"sketch": 0, "distance": 10}));
        let sketch = self.on_top();
        self.point(sketch, 30.0, 0.0);
        self.exec(
            "feature.hole",
            json!({"sketch": sketch, "diameter": 6, "depth": "through"}),
        );
    }
}

fn volume(state: &Value) -> f64 {
    state["body"]["volume"].as_f64().unwrap()
}

fn bound(state: &Value, end: &str, axis: &str) -> f64 {
    state["body"]["bounds"][end][axis].as_f64().unwrap()
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

const PLATE: f64 = 100.0 * 100.0 * 10.0;
const LOPSIDED: f64 = 150.0 * 100.0 * 10.0;

#[test]
fn a_mirrored_hole_lands_on_the_other_side_of_the_sketch_axis() {
    let mut s = Session::new();
    s.lopsided_plate_with_hole();
    let one = LOPSIDED - PI * 9.0 * 10.0;
    assert!((volume(&s.state_now()) - one).abs() < 1e-3);
    let state = s.exec("feature.mirror", json!({"feature": 1, "plane": "sketch_x"}));
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    // The hole at x = 30 now has a twin at x = -30, inside the plate.
    let two = LOPSIDED - 2.0 * PI * 9.0 * 10.0;
    assert!((volume(&state) - two).abs() < 1e-3, "{}", volume(&state));
    // Undo takes the twin away again.
    let state = s.undo();
    assert!((volume(&state) - one).abs() < 1e-3, "{}", volume(&state));
}

#[test]
fn the_other_axis_leaves_a_hole_on_the_axis_where_it_is() {
    let mut s = Session::new();
    s.lopsided_plate_with_hole();
    // The hole sits at y = 0, so mirroring in y lands it on itself: still one
    // hole's worth of material gone.
    let state = s.exec("feature.mirror", json!({"feature": 1, "plane": "sketch_y"}));
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    let one = LOPSIDED - PI * 9.0 * 10.0;
    assert!((volume(&state) - one).abs() < 1e-3, "{}", volume(&state));
}

#[test]
fn a_mirrored_extrude_doubles_the_part() {
    let mut s = Session::new();
    s.plate();
    let state = s.exec("feature.mirror", json!({"feature": 0, "plane": "model_z"}));
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    assert!(
        (volume(&state) - 2.0 * PLATE).abs() < 1e-3,
        "{}",
        volume(&state)
    );
    assert!((bound(&state, "min", "z") + 10.0).abs() < 1e-6);
    assert!((bound(&state, "max", "z") - 10.0).abs() < 1e-6);
    // The same through the sketch's own plane, by editing the mirror.
    let state = s.exec(
        "feature.edit_mirror",
        json!({"feature": 1, "plane": "sketch_z"}),
    );
    assert!((volume(&state) - 2.0 * PLATE).abs() < 1e-3);
    assert!((bound(&state, "min", "z") + 10.0).abs() < 1e-6);
}

#[test]
fn a_mirrored_revolve() {
    let mut s = Session::new();
    s.plate_of(-10.0);
    s.exec("sketch.create", json!({"plane": "xy", "name": "Ring"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 1, "from": {"x": 30, "y": 1}, "to": {"x": 40, "y": 4}}),
    );
    let state = s.exec(
        "feature.revolve",
        json!({"sketch": 1, "axis": "y", "angle": 40, "operation": "cut"}),
    );
    let one = PLATE - volume(&state);
    assert!(one > 1.0, "the revolve cut nothing: {one}");
    let state = s.exec("feature.mirror", json!({"feature": 1, "plane": "sketch_x"}));
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    let cut = PLATE - volume(&state);
    assert!(
        (cut - 2.0 * one).abs() < 1e-3,
        "two wedges cut {cut}, one cuts {one}"
    );
}

#[test]
fn names_survive_editing_the_original() {
    let mut s = Session::new();
    s.lopsided_plate_with_hole();
    let state = s.exec("feature.mirror", json!({"feature": 1, "plane": "sketch_x"}));
    let before: Vec<String> = face_names(&state)
        .into_iter()
        .filter(|n| n.contains("f2/"))
        .collect();
    assert!(!before.is_empty());
    let state = s.exec(
        "feature.edit_hole",
        json!({"feature": 1, "diameter": 8, "depth": "through"}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    let after: Vec<String> = face_names(&state)
        .into_iter()
        .filter(|n| n.contains("f2/"))
        .collect();
    assert_eq!(before, after);
    let expected = LOPSIDED - 2.0 * PI * 16.0 * 10.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{}",
        volume(&state)
    );
}

#[test]
fn bad_mirrors_say_what_is_wrong() {
    let mut s = Session::new();
    s.lopsided_plate_with_hole();
    let message = s.refused("feature.mirror", json!({"feature": 1, "plane": "diagonal"}));
    assert!(message.contains("unknown mirror plane"), "{message}");
    let message = s.refused("feature.mirror", json!({"feature": 1}));
    assert!(message.contains("plane"), "{message}");
    let message = s.refused("feature.mirror", json!({"feature": 9, "plane": "model_x"}));
    assert!(message.contains("feature"), "{message}");
    // A chamfer has no tool to mirror.
    let top = {
        let state = s.state_now();
        face_names(&state)
            .into_iter()
            .find(|n| n.starts_with("f0/end/"))
            .unwrap()
    };
    s.exec("feature.chamfer", json!({"faces": [top], "distance": 0.5}));
    let message = s.refused("feature.mirror", json!({"feature": 2, "plane": "model_x"}));
    assert!(
        message.contains("extrudes, holes and revolves"),
        "{message}"
    );
    // Editing something that is not a mirror.
    let message = s.refused(
        "feature.edit_mirror",
        json!({"feature": 1, "plane": "model_x"}),
    );
    assert!(message.contains("not a mirror"), "{message}");
    // A mirror whose source is deleted fails with the reason, and the rest of
    // the timeline still stands.
    s.exec("feature.mirror", json!({"feature": 1, "plane": "model_x"}));
    let state = s.exec("feature.delete", json!({"feature": 1}));
    let reason = status(&state, 3)["reason"].as_str().unwrap().to_owned();
    assert!(reason.contains("did not build"), "{reason}");
    assert_eq!(status(&state, 0)["state"], "ok");
}
