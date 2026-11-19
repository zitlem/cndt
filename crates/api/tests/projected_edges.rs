// SPDX-License-Identifier: AGPL-3.0-or-later
//! Model edges projected into a sketch through the JSON API (decision 0033):
//! a straight edge and a circular rim land in the right place with their
//! points fixed, refusals say why, a lost or moved edge fails the features
//! that use the sketch, undo, and the file round trip.

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

    /// A 100 × 100 × 10 plate with its base on z = 0 (sketch 0, feature 0),
    /// with a 20 mm hole through the middle (sketch 1, feature 1).
    fn drilled_plate(&mut self) {
        self.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        self.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": -50, "y": -50}, "to": {"x": 50, "y": 50}}),
        );
        self.exec("feature.extrude", json!({"sketch": 0, "distance": 10}));
        let top = self.top();
        self.exec("sketch.create", json!({"face": top, "name": "Bore"}));
        self.exec(
            "sketch.add_point",
            json!({"sketch": 1, "at": {"x": 0, "y": 0}}),
        );
        self.exec(
            "sketch.add_circle",
            json!({"sketch": 1, "center": 0, "radius": 10}),
        );
        self.exec(
            "feature.extrude",
            json!({"sketch": 1, "distance": -10, "operation": "cut"}),
        );
    }

    /// The plate's top face: the extrusion's end cap.
    fn top(&mut self) -> String {
        let state = self.state_now();
        named(&state, "f0/end/").remove(0)
    }
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

fn named(state: &Value, prefix: &str) -> Vec<String> {
    face_names(state)
        .into_iter()
        .filter(|n| n.starts_with(prefix))
        .collect()
}

/// The entities of a sketch, from the state.
fn entities(state: &Value, sketch: u32) -> Vec<Value> {
    let record = &state["document"]["sketches"][sketch.to_string()];
    let all = &record["sketch"]["entities"];
    match all {
        Value::Array(list) => list.clone(),
        Value::Object(map) => map.values().cloned().collect(),
        other => panic!("entities: {other}"),
    }
}

fn status(state: &Value, feature: u64) -> Value {
    state["feature_status"][feature.to_string()].clone()
}

#[test]
fn a_straight_edge_projects_to_a_fixed_line_of_the_right_length() {
    let mut s = Session::new();
    s.drilled_plate();
    let state = s.state_now();
    let top = s.top();
    let side = named(&state, "f0/side/").remove(0);
    s.exec("sketch.create", json!({"face": top, "name": "Outline"}));
    let state = s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": side}),
    );
    let list = entities(&state, 2);
    let lines: Vec<&Value> = list
        .iter()
        .filter(|e| e["geometry"]["type"] == "line")
        .collect();
    assert_eq!(lines.len(), 1, "{list:?}");
    let points: Vec<(f64, f64)> = list
        .iter()
        .filter(|e| e["geometry"]["type"] == "point")
        .map(|e| {
            (
                e["geometry"]["at"]["x"].as_f64().unwrap(),
                e["geometry"]["at"]["y"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(points.len(), 2, "{list:?}");
    let length = (points[0].0 - points[1].0).hypot(points[0].1 - points[1].1);
    assert!((length - 100.0).abs() < 1e-3, "length {length}");
    // Both ends are on the outline of the plate.
    for (x, y) in &points {
        assert!(
            (x.abs() - 50.0).abs() < 1e-3 || (y.abs() - 50.0).abs() < 1e-3,
            "({x}, {y})"
        );
    }
    let constraints = state["document"]["sketches"]["2"]["sketch"]["constraints"].to_string();
    assert_eq!(constraints.matches("\"fix\"").count(), 2, "{constraints}");
    assert_eq!(
        state["document"]["sketches"]["2"]["projections"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_circular_rim_projects_to_a_circle_with_its_radius() {
    let mut s = Session::new();
    s.drilled_plate();
    let state = s.state_now();
    let top = s.top();
    // The bore wall is named by the circle that made it; the rim is where it
    // meets the top.
    let wall = named(&state, "f1/side/")
        .into_iter()
        .next()
        .or_else(|| named(&state, "f1/").into_iter().next())
        .unwrap();
    s.exec("sketch.create", json!({"face": top, "name": "Rim"}));
    let state = s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": wall}),
    );
    let list = entities(&state, 2);
    let circle = list
        .iter()
        .find(|e| e["geometry"]["type"] == "circle")
        .unwrap_or_else(|| panic!("no circle: {list:?}"));
    let radius = circle["geometry"]["radius"].as_f64().unwrap();
    assert!((radius - 10.0).abs() < 1e-3, "radius {radius}");
}

#[test]
fn a_projected_edge_can_be_extruded_with_the_rest_of_the_sketch() {
    let mut s = Session::new();
    s.drilled_plate();
    let state = s.state_now();
    let top = s.top();
    let wall = named(&state, "f1/side/")
        .into_iter()
        .next()
        .or_else(|| named(&state, "f1/").into_iter().next())
        .unwrap();
    s.exec("sketch.create", json!({"face": top, "name": "Rim"}));
    s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": wall}),
    );
    let state = s.exec("feature.extrude", json!({"sketch": 2, "distance": 3}));
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    // A 3 mm collar the size of the bore fills the hole's top: the volume
    // goes up by its closed form, pi r^2 h.
    let expected = 100.0 * 100.0 * 10.0 - std::f64::consts::PI * 100.0 * 10.0
        + std::f64::consts::PI * 100.0 * 3.0;
    let volume = state["body"]["volume"].as_f64().unwrap();
    assert!((volume - expected).abs() < 1.0, "{volume} vs {expected}");
}

#[test]
fn projecting_is_refused_with_a_reason_and_leaves_the_sketch_alone() {
    let mut s = Session::new();
    s.drilled_plate();
    let state = s.state_now();
    let top = s.top();
    let side = named(&state, "f0/side/").remove(0);
    let sides = named(&state, "f0/side/");
    s.exec("sketch.create", json!({"face": top, "name": "Outline"}));
    // Two faces that do not meet.
    let why = s.refused(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": "f0/start/4-5-6-7"}),
    );
    assert!(why.contains("no longer meet"), "{why}");
    // A face that is not there.
    let why = s.refused(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": "f9/side/e0"}),
    );
    assert!(why.contains("no longer exists"), "{why}");
    // A sketch that is not there.
    let why = s.refused(
        "sketch.project_edge",
        json!({"sketch": 9, "first": top, "second": side}),
    );
    assert!(why.contains("does not exist"), "{why}");
    // Of the other three sides, the two next to this one meet it at an edge
    // square to the plane of the top, which projects to a point.
    let mut square = 0;
    for other in &sides[1..] {
        let why = s.refused(
            "sketch.project_edge",
            json!({"sketch": 2, "first": side, "second": other}),
        );
        if why.contains("projects to a point") {
            square += 1;
        } else {
            assert!(why.contains("no longer meet"), "{why}");
        }
    }
    assert_eq!(square, 2);
    assert!(entities(&s.state_now(), 2).is_empty());
    // The same edge twice.
    s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": side}),
    );
    let why = s.refused(
        "sketch.project_edge",
        json!({"sketch": 2, "first": side, "second": top}),
    );
    assert!(why.contains("already projected"), "{why}");
}

#[test]
fn a_shape_sent_by_the_caller_is_replaced_by_the_kernel() {
    let mut s = Session::new();
    s.drilled_plate();
    let top = s.top();
    let side = named(&s.state_now(), "f0/side/").remove(0);
    s.exec("sketch.create", json!({"face": top, "name": "Outline"}));
    let state = s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": side,
               "shape": {"type": "line", "start": {"x": 0, "y": 0}, "end": {"x": 1, "y": 0}}}),
    );
    let longest = entities(&state, 2)
        .iter()
        .filter(|e| e["geometry"]["type"] == "point")
        .map(|e| {
            e["geometry"]["at"]["x"]
                .as_f64()
                .unwrap()
                .abs()
                .max(e["geometry"]["at"]["y"].as_f64().unwrap().abs())
        })
        .fold(0.0, f64::max);
    assert!((longest - 50.0).abs() < 1e-3, "{longest}");
}

#[test]
fn undo_takes_the_projection_away() {
    let mut s = Session::new();
    s.drilled_plate();
    let top = s.top();
    let side = named(&s.state_now(), "f0/side/").remove(0);
    s.exec("sketch.create", json!({"face": top, "name": "Outline"}));
    s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": side}),
    );
    let state = s.undo();
    assert!(entities(&state, 2).is_empty());
    assert!(
        state["document"]["sketches"]["2"]
            .get("projections")
            .is_none()
    );
}

#[test]
fn a_projected_edge_that_has_moved_fails_the_features_that_use_it() {
    let mut s = Session::new();
    s.drilled_plate();
    let state = s.state_now();
    let top = s.top();
    let wall = named(&state, "f1/side/")
        .into_iter()
        .next()
        .or_else(|| named(&state, "f1/").into_iter().next())
        .unwrap();
    s.exec("sketch.create", json!({"face": top, "name": "Rim"}));
    s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": wall}),
    );
    let state = s.exec("feature.extrude", json!({"sketch": 2, "distance": 3}));
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    // Move the bore: its rim moves, so the stored circle is out of date and
    // the collar is not built from it.
    let state = s.exec(
        "sketch.move_point",
        json!({"sketch": 1, "point": 0, "to": {"x": 5, "y": 0}}),
    );
    assert_eq!(status(&state, 2)["state"], "failed", "{state}");
    let reason = status(&state, 2)["reason"].as_str().unwrap().to_owned();
    assert!(
        reason.contains("Rim") && reason.contains("out of date"),
        "{reason}"
    );
    // Put it back and the collar builds again.
    let state = s.exec(
        "sketch.move_point",
        json!({"sketch": 1, "point": 0, "to": {"x": 0, "y": 0}}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
}

#[test]
fn a_lost_edge_fails_the_features_that_use_it_and_says_so() {
    let mut s = Session::new();
    s.drilled_plate();
    let state = s.state_now();
    let top = s.top();
    let wall = named(&state, "f1/side/")
        .into_iter()
        .next()
        .or_else(|| named(&state, "f1/").into_iter().next())
        .unwrap();
    s.exec("sketch.create", json!({"face": top, "name": "Rim"}));
    s.exec(
        "sketch.project_edge",
        json!({"sketch": 2, "first": top, "second": wall}),
    );
    s.exec("feature.extrude", json!({"sketch": 2, "distance": 3}));
    // Delete the bore: the wall and its rim are gone.
    let state = s.exec("feature.delete", json!({"feature": 1}));
    assert_eq!(
        status(&state, 2)["state"],
        "failed",
        "{}",
        state["feature_status"]
    );
    let reason = status(&state, 2)["reason"].as_str().unwrap().to_owned();
    assert!(
        reason.contains("Rim") && reason.contains("no longer"),
        "{reason}"
    );
}
