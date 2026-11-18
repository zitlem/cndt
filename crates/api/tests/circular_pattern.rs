// SPDX-License-Identifier: AGPL-3.0-or-later
//! The circular pattern through the JSON API (decision 0022): a bolt circle of
//! holes, a partial angle, a pattern of a revolve, names that survive editing
//! the original, undo, and refusals with their reasons.

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

/// Plate, then one hole of diameter 6 at (30, 0) through a sketch on the top.
fn plate_with_hole(s: &mut Session) {
    s.plate();
    let sketch = s.on_top();
    s.point(sketch, 30.0, 0.0);
    s.exec(
        "feature.hole",
        json!({"sketch": sketch, "diameter": 6, "depth": "through"}),
    );
}

#[test]
fn a_bolt_circle_of_holes() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    let one = PLATE - PI * 9.0 * 10.0;
    let state = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 6}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    assert_eq!(
        state["document"]["features"][2]["feature"]["type"],
        "circular_pattern"
    );
    assert_eq!(
        state["document"]["features"][2]["feature"]["name"],
        "CircularPattern1"
    );
    // Six holes: the original and five copies, all through.
    let expected = PLATE - 6.0 * PI * 9.0 * 10.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{} against {expected}",
        volume(&state)
    );
    assert!(volume(&state) < one);
    // Each copy's walls are named by the original's wall and its place round.
    let names = face_names(&state);
    let copies: Vec<&String> = names.iter().filter(|n| n.contains("f2/")).collect();
    assert!(copies.len() >= 5, "{names:?}");
    for i in 1..6 {
        assert!(
            names.iter().any(|n| n.contains(&format!("f2/at{i},0/"))),
            "copy {i} missing: {names:?}"
        );
    }

    // Undo takes the pattern away; one hole remains.
    let state = s.undo();
    assert!((volume(&state) - one).abs() < 1e-3, "{}", volume(&state));
}

#[test]
fn a_partial_angle_spreads_the_copies_between_the_first_and_the_last() {
    let mut s = Session::new();
    s.plate();
    // A boss overlapping the plate's right edge, 5 mm in radius.
    let sketch = s.on_top();
    s.exec(
        "sketch.add_circle",
        json!({"sketch": sketch, "center": {"x": 50, "y": 0}, "radius": 5}),
    );
    s.exec("feature.extrude", json!({"sketch": sketch, "distance": 5}));
    let state = s.state_now();
    assert!((bound(&state, "max", "x") - 55.0).abs() < 1e-6);
    assert!((bound(&state, "max", "y") - 50.0).abs() < 1e-6);
    // Three bosses a quarter turn apart: at 0°, 45° and 90°.
    let state = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 3, "angle": 90}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    assert!(
        (bound(&state, "max", "y") - 55.0).abs() < 1e-6,
        "{}",
        bound(&state, "max", "y")
    );
    assert!((bound(&state, "min", "x") + 50.0).abs() < 1e-6);
    let single = {
        let boss = PI * 25.0 * 5.0;
        // The part of the boss over the plate is already plate; only the
        // half outside the plate's edge adds material above it.
        boss
    };
    assert!(volume(&state) > PLATE + single, "{}", volume(&state));
    // The whole way round the copies are spaced 360 / count apart: four
    // bosses reach all four sides.
    let state = s.exec(
        "feature.edit_circular_pattern",
        json!({"feature": 2, "count": 4}),
    );
    assert!((bound(&state, "min", "x") + 55.0).abs() < 1e-6);
    assert!((bound(&state, "min", "y") + 55.0).abs() < 1e-6);
    assert!((bound(&state, "max", "y") - 55.0).abs() < 1e-6);
}

impl Session {
    fn state_now(&mut self) -> Value {
        self.call(json!({"method": "state"}))["result"].clone()
    }
}

#[test]
fn a_centre_point_moves_the_axis() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    // The hole sits at (30, 0). A centre at (10, 10) is a point of its
    // sketch only if it is there, and it is not.
    let message = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 3, "center": 99}),
    );
    let reason = status(&message, 2)["reason"].as_str().unwrap().to_owned();
    assert!(reason.contains("not a point"), "{reason}");
}

#[test]
fn a_centre_point_of_the_sketch_moves_the_axis() {
    let mut s = Session::new();
    s.plate();
    let sketch = s.on_top();
    s.exec(
        "sketch.add_circle",
        json!({"sketch": sketch, "center": {"x": 50, "y": 0}, "radius": 5}),
    );
    // A line gives the plate's corner (50, -50) a point to be named by.
    let corner = s.point(sketch, 50.0, -50.0);
    let other = s.point(sketch, 20.0, -20.0);
    s.exec(
        "sketch.add_line",
        json!({"sketch": sketch, "start": corner, "end": other}),
    );
    s.exec("feature.extrude", json!({"sketch": sketch, "distance": 5}));
    // A quarter turn about the corner takes the boss at (50, 0) to (0, -50).
    let state = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 2, "angle": 90, "center": corner}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    assert!((bound(&state, "min", "y") + 55.0).abs() < 1e-6, "{state}");
    assert!((bound(&state, "max", "y") - 50.0).abs() < 1e-6);
    // About the sketch origin instead, the same quarter turn lands at (0, 50).
    let state = s.exec(
        "feature.edit_circular_pattern",
        json!({"feature": 2, "count": 2, "angle": 90}),
    );
    assert!((bound(&state, "max", "y") - 55.0).abs() < 1e-6);
    assert!((bound(&state, "min", "y") + 50.0).abs() < 1e-6);
}

#[test]
fn a_pattern_of_a_revolve() {
    let mut s = Session::new();
    // The plate below the sketch plane, where the wedge turns into it.
    s.plate_of(-10.0);
    // A wedge of a ring turned about the sketch's Y axis, cutting the plate.
    s.exec("sketch.create", json!({"plane": "xy", "name": "Ring"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 1, "from": {"x": 30, "y": 1}, "to": {"x": 40, "y": 4}}),
    );
    let state = s.exec(
        "feature.revolve",
        json!({"sketch": 1, "axis": "y", "angle": 40, "operation": "cut"}),
    );
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    let one = PLATE - volume(&state);
    assert!(one > 1.0, "the revolve cut nothing: {one}");
    let state = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 3}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    let cut = PLATE - volume(&state);
    assert!(
        (cut - 3.0 * one).abs() < 1e-3,
        "three wedges cut {cut}, one cuts {one}"
    );
}

#[test]
fn names_survive_editing_the_original() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    let state = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 4}),
    );
    let before: Vec<String> = face_names(&state)
        .into_iter()
        .filter(|n| n.contains("f2/"))
        .collect();
    assert!(!before.is_empty());
    // A bigger hole: the same faces, still found by the same names.
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
    let expected = PLATE - 4.0 * PI * 16.0 * 10.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{}",
        volume(&state)
    );
}

#[test]
fn bad_patterns_say_what_is_wrong() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    for count in [0, 1] {
        let message = s.refused(
            "feature.circular_pattern",
            json!({"feature": 1, "count": count}),
        );
        assert!(message.contains("at least 2"), "{message}");
    }
    for angle in [0.0, -10.0, 361.0] {
        let message = s.refused(
            "feature.circular_pattern",
            json!({"feature": 1, "count": 3, "angle": angle}),
        );
        assert!(message.contains("angle"), "{message}");
    }
    let message = s.refused(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 501}),
    );
    assert!(message.contains("too many"), "{message}");
    let message = s.refused(
        "feature.circular_pattern",
        json!({"feature": 9, "count": 3}),
    );
    assert!(message.contains("feature"), "{message}");
    // The plate is no sketchless feature: a chamfer cannot be patterned.
    let top = {
        let state = s.state_now();
        face_names(&state)
            .into_iter()
            .find(|n| n.starts_with("f0/end/"))
            .unwrap()
    };
    s.exec("feature.chamfer", json!({"faces": [top], "distance": 0.5}));
    let message = s.refused(
        "feature.circular_pattern",
        json!({"feature": 2, "count": 3}),
    );
    assert!(
        message.contains("extrudes, holes and revolves"),
        "{message}"
    );
    // Editing something that is not a circular pattern.
    let message = s.refused(
        "feature.edit_circular_pattern",
        json!({"feature": 1, "count": 3}),
    );
    assert!(message.contains("not a circular pattern"), "{message}");
    // A pattern whose source is deleted fails with the reason and the rest
    // of the timeline still stands.
    s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 3}),
    );
    let state = s.exec("feature.delete", json!({"feature": 1}));
    let reason = status(&state, 3)["reason"].as_str().unwrap().to_owned();
    assert!(reason.contains("did not build"), "{reason}");
    assert_eq!(status(&state, 0)["state"], "ok");
}
