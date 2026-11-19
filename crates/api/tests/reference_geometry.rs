// SPDX-License-Identifier: AGPL-3.0-or-later
//! Work planes, work axes and work points through the JSON API (decision
//! 0029): a sketch on an offset plane, a revolve and a circular pattern about
//! a work axis, refusals with their reasons, undo.

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

    /// A 100 × 100 × 10 plate centred on the origin: sketch 0, feature 0.
    fn plate(&mut self) -> Value {
        self.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        self.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": -50, "y": -50}, "to": {"x": 50, "y": 50}}),
        );
        self.exec("feature.extrude", json!({"sketch": 0, "distance": 10}))
    }
}

fn volume(state: &Value) -> f64 {
    state["body"]["volume"].as_f64().unwrap()
}

fn bound(state: &Value, end: &str, axis: &str) -> f64 {
    state["body"]["bounds"][end][axis].as_f64().unwrap()
}

fn status(state: &Value, feature: u64) -> Value {
    state["feature_status"][feature.to_string()].clone()
}

const PLATE: f64 = 100.0 * 100.0 * 10.0;

#[test]
fn a_sketch_on_an_offset_work_plane_builds_where_the_plane_is() {
    let mut s = Session::new();
    s.plate();
    let state = s.exec(
        "reference.add_work_plane",
        json!({"plane": "xy", "offset": 10}),
    );
    assert_eq!(state["document"]["references"]["0"]["name"], "Plane1");
    s.exec("sketch.create", json!({"work_plane": 0}));
    let state = s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 1, "from": {"x": -10, "y": -10}, "to": {"x": 10, "y": 10}}),
    );
    // The sketch is placed on the plane: its origin is at z = 10.
    assert_eq!(state["sketch_frames"]["1"]["frame"]["origin"]["z"], 10.0);
    let state = s.exec("feature.extrude", json!({"sketch": 1, "distance": 5}));
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    assert!((volume(&state) - (PLATE + 400.0 * 5.0)).abs() < 1e-3);
    assert!((bound(&state, "max", "z") - 15.0).abs() < 1e-6);
}

#[test]
fn a_negative_offset_and_a_cut_from_a_work_plane() {
    let mut s = Session::new();
    s.plate();
    s.exec(
        "reference.add_work_plane",
        json!({"plane": "xy", "offset": 4, "name": "Mid"}),
    );
    s.exec("sketch.create", json!({"work_plane": 0}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 1, "from": {"x": -10, "y": -10}, "to": {"x": 10, "y": 10}}),
    );
    let state = s.exec(
        "feature.extrude",
        json!({"sketch": 1, "distance": 3, "operation": "cut"}),
    );
    assert_eq!(status(&state, 1)["state"], "ok", "{state}");
    assert!((volume(&state) - (PLATE - 400.0 * 3.0)).abs() < 1e-3);
    // Below the origin plane: the plane lies at z = -2, under the plate.
    s.exec(
        "reference.add_work_plane",
        json!({"plane": "xy", "offset": -2}),
    );
    let state = s.exec("sketch.create", json!({"work_plane": 1}));
    assert_eq!(state["sketch_frames"]["2"]["frame"]["origin"]["z"], -2.0);
}

#[test]
fn a_revolve_about_a_work_axis_through_a_point() {
    let mut s = Session::new();
    s.exec("sketch.create", json!({"plane": "xy", "name": "Section"}));
    // x from 10 to 20, y from 0 to 5; the axis is the line x = 5 along Y, so
    // the ring is 5 to 15 from the axis.
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 10, "y": 0}, "to": {"x": 20, "y": 5}}),
    );
    s.exec(
        "reference.add_work_axis",
        json!({"x": 5, "dy": 1, "name": "Spindle"}),
    );
    let state = s.exec("feature.revolve", json!({"sketch": 0, "axis_work": 0}));
    assert_eq!(status(&state, 0)["state"], "ok", "{state}");
    let expected = PI * (15.0_f64.powi(2) - 5.0_f64.powi(2)) * 5.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{} against {expected}",
        volume(&state)
    );
    assert_eq!(
        state["document"]["features"][0]["feature"]["axis"],
        json!({"type": "work", "reference": 0})
    );
}

#[test]
fn a_work_axis_outside_the_sketch_plane_fails_the_revolve_with_the_reason() {
    let mut s = Session::new();
    s.exec("sketch.create", json!({"plane": "xy"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 10, "y": 0}, "to": {"x": 20, "y": 5}}),
    );
    // Along Z: it points out of the sketch plane.
    s.exec("reference.add_work_axis", json!({"dz": 1}));
    // Along Y but 3 mm above the plane.
    s.exec("reference.add_work_axis", json!({"z": 3, "dy": 1}));
    for axis in [0, 1] {
        let state = s.exec("feature.revolve", json!({"sketch": 0, "axis_work": axis}));
        let failed = status(&state, axis);
        assert_eq!(failed["state"], "failed", "{state}");
        let reason = failed["reason"].as_str().unwrap();
        assert!(
            reason.contains("does not lie in the sketch's plane"),
            "{reason}"
        );
    }
}

#[test]
fn a_circular_pattern_about_a_work_axis_away_from_the_sketch_origin() {
    let mut s = Session::new();
    let state = s.plate();
    let top = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|n| n.starts_with("f0/end/"))
        .unwrap()
        .to_owned();
    s.exec("sketch.create", json!({"face": top}));
    s.exec(
        "sketch.add_point",
        json!({"sketch": 1, "at": {"x": 30, "y": 0}}),
    );
    s.exec(
        "feature.hole",
        json!({"sketch": 1, "diameter": 6, "depth": "through"}),
    );
    // Half a turn about the vertical line through (10, 0): the hole at x = 30
    // lands at x = -10.
    s.exec(
        "reference.add_work_axis",
        json!({"x": 10, "dz": 1, "name": "Pivot"}),
    );
    let state = s.exec(
        "feature.circular_pattern",
        json!({"feature": 1, "count": 2, "angle": 180, "axis_work": 0}),
    );
    assert_eq!(status(&state, 2)["state"], "ok", "{state}");
    let expected = PLATE - 2.0 * PI * 9.0 * 10.0;
    assert!((volume(&state) - expected).abs() < 1e-3);
    // The copy's wall is named as a copy of the original's.
    let hole_faces = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .filter(|n| n.starts_with("f2/at1,0/"))
        .count();
    assert!(hole_faces > 0, "no face is named as a copy: {state}");
    // Editing it back to the sketch normal about a centre is refused if both
    // are given.
    let message = s.refused(
        "feature.edit_circular_pattern",
        json!({"feature": 2, "count": 2, "angle": 180, "axis_work": 0, "center": 1}),
    );
    assert!(message.contains("not both"), "{message}");
}

#[test]
fn reference_geometry_in_use_cannot_be_deleted_and_undo_works() {
    let mut s = Session::new();
    s.exec(
        "reference.add_work_plane",
        json!({"plane": "xz", "offset": 7}),
    );
    s.exec("reference.add_work_point", json!({"x": 1, "y": 2, "z": 3}));
    s.exec(
        "sketch.create",
        json!({"work_plane": 0, "name": "Floating"}),
    );
    let message = s.refused("reference.delete", json!({"reference": 0}));
    assert!(
        message.contains("Plane1") && message.contains("Floating"),
        "{message}"
    );
    // The point is not used.
    let state = s.exec("reference.delete", json!({"reference": 1}));
    assert!(state["document"]["references"]["1"].is_null());
    let state = s.undo();
    assert_eq!(state["document"]["references"]["1"]["type"], "work_point");
    assert_eq!(
        state["document"]["references"]["1"]["position"],
        json!([1.0, 2.0, 3.0])
    );
    // Once the sketch is gone the plane can go too.
    s.exec("sketch.delete", json!({"sketch": 0}));
    let state = s.exec("reference.delete", json!({"reference": 0}));
    assert!(state["document"]["references"]["0"].is_null());
    let state = s.undo();
    assert_eq!(state["document"]["references"]["0"]["type"], "work_plane");
}

#[test]
fn refusals_say_what_is_wrong() {
    let mut s = Session::new();
    s.exec(
        "reference.add_work_plane",
        json!({"plane": "xy", "offset": 1}),
    );
    s.exec("reference.add_work_axis", json!({"dx": 1}));
    let cases = [
        (
            "reference.add_work_axis",
            json!({"x": 1}),
            "direction that is not zero",
        ),
        (
            "reference.add_work_plane",
            json!({"plane": "ab", "offset": 1}),
            "unknown plane",
        ),
        ("sketch.create", json!({"work_plane": 9}), "does not exist"),
        (
            "sketch.create",
            json!({"work_plane": 1}),
            "is a work axis, not a work plane",
        ),
        (
            "sketch.create",
            json!({"work_plane": 0, "plane": "xy"}),
            "and only one",
        ),
        (
            "reference.delete",
            json!({"reference": 9}),
            "does not exist",
        ),
    ];
    for (command, args, expected) in cases {
        let message = s.refused(command, args.clone());
        assert!(message.contains(expected), "{command} {args}: {message}");
    }
    // A revolve about a plane is refused when it is made.
    s.exec("sketch.create", json!({"plane": "xy"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 10, "y": 0}, "to": {"x": 20, "y": 5}}),
    );
    let message = s.refused("feature.revolve", json!({"sketch": 0, "axis_work": 0}));
    assert!(message.contains("not a work axis"), "{message}");
    let message = s.refused(
        "feature.revolve",
        json!({"sketch": 0, "axis_work": 1, "axis": "x"}),
    );
    assert!(message.contains("not several"), "{message}");
}
