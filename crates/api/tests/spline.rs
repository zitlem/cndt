// SPDX-License-Identifier: AGPL-3.0-or-later
//! Splines in sketches through the JSON API (decision 0034): a closed spline
//! is a region that extrudes to about the closed-form volume, a spline joins
//! lines into one profile, refusals say why, and undo, mirror, delete and
//! the file round trip keep working.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cndt_api::Api;
use serde_json::{Value, json};

struct Session {
    api: Api,
}

impl Session {
    fn new() -> Self {
        let mut session = Self { api: Api::new() };
        session.exec("sketch.create", json!({"plane": "xy", "name": "Outline"}));
        session
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

    fn state(&mut self) -> Value {
        self.call(json!({"method": "state"}))["result"].clone()
    }

    /// The entities of sketch 0, by id.
    fn entities(&mut self) -> Vec<Value> {
        let state = self.state();
        state["document"]["sketches"]["0"]["sketch"]["entities"]
            .as_object()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }

    fn volume(&mut self) -> f64 {
        self.state()["body"]["volume"].as_f64().unwrap()
    }
}

/// `n` positions evenly round a circle of radius `r` about the origin.
fn ring(n: usize, r: f64) -> Vec<Value> {
    (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            json!({"x": r * a.cos(), "y": r * a.sin()})
        })
        .collect()
}

#[test]
fn a_closed_spline_extrudes_to_the_volume_of_the_shape_it_follows() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": ring(12, 10.0), "closed": true}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 5}));
    let expected = std::f64::consts::PI * 100.0 * 5.0;
    let volume = s.volume();
    assert!(
        (volume - expected).abs() / expected < 0.005,
        "volume {volume} against {expected}"
    );
}

#[test]
fn a_spline_and_a_line_close_one_profile() {
    // A half-lens: a spline arching from (0, 0) to (20, 0) over a straight base.
    let mut s = Session::new();
    s.exec(
        "sketch.add_point",
        json!({"sketch": 0, "at": {"x": 0, "y": 0}}),
    );
    s.exec(
        "sketch.add_point",
        json!({"sketch": 0, "at": {"x": 20, "y": 0}}),
    );
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": [0, {"x": 10, "y": 6}, 1]}),
    );
    s.exec(
        "sketch.add_line",
        json!({"sketch": 0, "start": 0, "end": 1}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 2}));
    let volume = s.volume();
    // The arch rises about 6 mm over a 20 mm base: between a triangle (60)
    // and the full rectangle (120) of the same height, times the depth 2.
    assert!(volume > 2.0 * 60.0 && volume < 2.0 * 120.0, "{volume}");
}

#[test]
fn a_spline_loop_with_a_hole_cuts_the_hole() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": ring(12, 10.0), "closed": true}),
    );
    s.exec(
        "sketch.add_point",
        json!({"sketch": 0, "at": {"x": 0, "y": 0}}),
    );
    s.exec(
        "sketch.add_circle",
        json!({"sketch": 0, "center": 13, "radius": 3}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 4}));
    let expected = std::f64::consts::PI * (100.0 - 9.0) * 4.0;
    let volume = s.volume();
    assert!(
        (volume - expected).abs() / expected < 0.005,
        "volume {volume} against {expected}"
    );
}

#[test]
fn a_spline_with_too_few_points_is_refused_and_leaves_nothing() {
    let mut s = Session::new();
    let message = s.refused(
        "sketch.add_spline",
        json!({"sketch": 0, "through": [{"x": 0, "y": 0}, {"x": 5, "y": 5}]}),
    );
    assert!(message.contains("at least 3"), "{message}");
    assert!(s.entities().is_empty());
}

#[test]
fn bad_spline_arguments_say_what_is_wanted() {
    let mut s = Session::new();
    let message = s.refused(
        "sketch.add_spline",
        json!({"sketch": 0, "through": [{"x": 0, "y": 0}, "a", {"x": 5, "y": 5}]}),
    );
    assert!(message.contains("point ids"), "{message}");
    let message = s.refused("sketch.add_spline", json!({"sketch": 0, "through": 3}));
    assert!(message.contains("list of points"), "{message}");
}

#[test]
fn a_spline_is_not_trimmed_or_offset() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": [{"x": 0, "y": 0}, {"x": 5, "y": 4}, {"x": 10, "y": 0}]}),
    );
    // Points 0..=2, the spline is entity 3.
    let message = s.refused(
        "sketch.trim",
        json!({"sketch": 0, "entity": 3, "at": {"x": 5, "y": 4}}),
    );
    assert!(message.contains("spline"), "{message}");
    let message = s.refused(
        "sketch.offset",
        json!({"sketch": 0, "entities": [3], "distance": 1, "toward": {"x": 5, "y": 0}}),
    );
    assert!(message.contains("spline"), "{message}");
}

#[test]
fn undo_removes_the_spline_and_its_points() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": ring(5, 4.0), "closed": true}),
    );
    assert_eq!(s.entities().len(), 6);
    let reply = s.call(json!({"method": "undo"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(s.entities().is_empty());
}

#[test]
fn deleting_a_spline_removes_the_points_nothing_else_uses() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": ring(4, 4.0), "closed": true}),
    );
    // Points 0..=3, the spline is entity 4.
    s.exec("sketch.remove_entity", json!({"sketch": 0, "entity": 4}));
    assert!(s.entities().is_empty());
}

#[test]
fn a_mirrored_spline_is_a_spline_through_mirrored_points() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": [{"x": 2, "y": 0}, {"x": 5, "y": 4}, {"x": 8, "y": 0}]}),
    );
    s.exec(
        "sketch.add_line",
        json!({"sketch": 0, "start": {"x": 0, "y": -5}, "end": {"x": 0, "y": 5}}),
    );
    // Points 0..=2, spline 3, the axis end points 4 and 5, the axis 6.
    s.exec(
        "sketch.mirror",
        json!({"sketch": 0, "entities": [3], "axis": 6}),
    );
    let splines = s
        .entities()
        .iter()
        .filter(|e| e["geometry"]["type"] == "spline")
        .count();
    assert_eq!(splines, 2);
}

#[test]
fn a_project_with_a_spline_survives_save_and_open() {
    let mut s = Session::new();
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": ring(8, 6.0), "closed": true}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 3}));
    let before = s.volume();
    let dir = std::env::temp_dir().join(format!("cndt-spline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("spline.cndt");
    let saved = s.call(json!({"method": "save_as", "path": path.to_str().unwrap()}));
    assert_eq!(saved["ok"], true, "{saved}");
    let mut again = Session { api: Api::new() };
    let opened = again.call(json!({"method": "open", "path": path.to_str().unwrap()}));
    assert_eq!(opened["ok"], true, "{opened}");
    let after = again.volume();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!((before - after).abs() < 1e-6, "{before} against {after}");
}

#[test]
fn a_spline_start_can_be_made_tangent_to_a_line() {
    // Points 0 and 1 are the ends of the base, 2 the far end of a diagonal.
    let mut s = Session::new();
    for at in [
        json!({"x": 0, "y": 0}),
        json!({"x": 20, "y": 0}),
        json!({"x": 8, "y": 8}),
    ] {
        s.exec("sketch.add_point", json!({"sketch": 0, "at": at}));
    }
    s.exec(
        "sketch.add_spline",
        json!({"sketch": 0, "through": [0, {"x": 10, "y": 6}, 1]}),
    );
    s.exec(
        "sketch.add_line",
        json!({"sketch": 0, "start": 0, "end": 1}),
    );
    s.exec(
        "sketch.add_line",
        json!({"sketch": 0, "start": 0, "end": 2}),
    );
    let fix = |point: u64| json!({"sketch": 0, "constraint": {"type": "fix", "point": point}});
    s.exec("sketch.constrain", fix(0));
    s.exec("sketch.constrain", fix(2));
    let tangent = |end: &str| {
        json!({"sketch": 0, "constraint": {
            "type": "spline_tangent", "spline": 4, "end": end, "to": 6}})
    };
    s.exec("sketch.constrain", tangent("start"));
    // The point after the start (id 3) now lies on the diagonal y = x.
    let state = s.state();
    let point = &state["document"]["sketches"]["0"]["sketch"]["entities"]["3"]["geometry"]["at"];
    let (x, y) = (point["x"].as_f64().unwrap(), point["y"].as_f64().unwrap());
    assert!((x - y).abs() < 1e-6, "({x}, {y})");
    // The diagonal does not touch the spline's end, so that is refused.
    let message = s.refused("sketch.constrain", tangent("end"));
    assert!(message.contains("share that point"), "{message}");
}
