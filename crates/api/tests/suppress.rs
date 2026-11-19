// SPDX-License-Identifier: AGPL-3.0-or-later
//! Suppressing and reordering features through the JSON API (decision
//! 0028): a suppressed feature leaves the part and its names, a pattern of
//! it fails with the reason, unsuppress and undo bring it back, and an order
//! that breaks a dependency is refused.

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

    /// A 100 × 100 × 10 plate: feature 0; a hole of diameter 6 through its
    /// top: feature 1.
    fn plate_with_hole(&mut self) {
        self.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        self.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": -50, "y": -50}, "to": {"x": 50, "y": 50}}),
        );
        let state = self.exec("feature.extrude", json!({"sketch": 0, "distance": 10}));
        let top = face_names(&state)
            .into_iter()
            .find(|n| n.starts_with("f0/end/"))
            .unwrap();
        self.exec("sketch.create", json!({"face": top}));
        self.exec(
            "sketch.add_point",
            json!({"sketch": 1, "at": {"x": -20, "y": 0}}),
        );
        self.exec(
            "feature.hole",
            json!({"sketch": 1, "diameter": 6, "depth": "through"}),
        );
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

fn order(state: &Value) -> Vec<u64> {
    state["document"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_u64().unwrap())
        .collect()
}

const PLATE: f64 = 100.0 * 100.0 * 10.0;
const HOLE: f64 = PI * 9.0 * 10.0;

#[test]
fn a_suppressed_hole_leaves_the_part_and_its_names_and_comes_back() {
    let mut s = Session::new();
    s.plate_with_hole();
    let state = s.exec("feature.suppress", json!({"feature": 1}));
    assert_eq!(status(&state, 1)["state"], "suppressed");
    assert_eq!(status(&state, 0)["state"], "ok");
    assert!((volume(&state) - PLATE).abs() < 1e-6, "{}", volume(&state));
    assert!(
        face_names(&state).iter().all(|n| !n.starts_with("f1/")),
        "{:?}",
        face_names(&state)
    );
    assert_eq!(state["document"]["features"][1]["suppressed"], true);
    let state = s.exec(
        "feature.suppress",
        json!({"feature": 1, "suppressed": false}),
    );
    assert_eq!(status(&state, 1)["state"], "ok");
    assert!((volume(&state) - (PLATE - HOLE)).abs() < 1e-6);
    assert!(face_names(&state).iter().any(|n| n.starts_with("f1/")));
    assert!(state["document"]["features"][1].get("suppressed").is_none());
}

#[test]
fn suppress_undoes_and_redoes() {
    let mut s = Session::new();
    s.plate_with_hole();
    s.exec("feature.suppress", json!({"feature": 1}));
    let state = s.undo();
    assert_eq!(status(&state, 1)["state"], "ok");
    assert!((volume(&state) - (PLATE - HOLE)).abs() < 1e-6);
    let reply = s.call(json!({"method": "redo"}));
    assert_eq!(
        reply["result"]["feature_status"]["1"]["state"],
        "suppressed"
    );
}

#[test]
fn a_pattern_of_a_suppressed_feature_fails_with_the_reason() {
    let mut s = Session::new();
    s.plate_with_hole();
    s.exec(
        "feature.pattern",
        json!({"feature": 1, "count_x": 2, "spacing_x": 30, "count_y": 1, "spacing_y": 0}),
    );
    let state = s.exec("feature.suppress", json!({"feature": 1}));
    assert_eq!(status(&state, 1)["state"], "suppressed");
    assert_eq!(status(&state, 2)["state"], "failed");
    let reason = status(&state, 2)["reason"].as_str().unwrap().to_owned();
    assert!(reason.contains("suppressed"), "{reason}");
    // The rest of the part still builds.
    assert!((volume(&state) - PLATE).abs() < 1e-6);
    let state = s.exec(
        "feature.suppress",
        json!({"feature": 1, "suppressed": false}),
    );
    assert_eq!(status(&state, 2)["state"], "ok");
    assert!((volume(&state) - (PLATE - 2.0 * HOLE)).abs() < 1e-6);
}

#[test]
fn a_feature_before_its_source_or_its_face_is_refused() {
    let mut s = Session::new();
    s.plate_with_hole();
    s.exec(
        "feature.pattern",
        json!({"feature": 1, "count_x": 2, "spacing_x": 30, "count_y": 1, "spacing_y": 0}),
    );
    // The pattern may not come before the hole it repeats.
    let message = s.refused("feature.move", json!({"feature": 2, "position": 1}));
    assert!(message.contains("must stay before"), "{message}");
    // The hole is sketched on a face of the plate: it may not come first.
    let message = s.refused("feature.move", json!({"feature": 1, "position": 0}));
    assert!(message.contains("face of"), "{message}");
    // The plate may not move after its dependents either.
    let message = s.refused("feature.move", json!({"feature": 0, "position": 2}));
    assert!(message.contains("must stay before"), "{message}");
    let message = s.refused("feature.move", json!({"feature": 1, "position": 9}));
    assert!(message.contains("no position 9"), "{message}");
    // Nothing changed.
    let state = s.exec(
        "feature.suppress",
        json!({"feature": 1, "suppressed": false}),
    );
    assert_eq!(order(&state), [0, 1, 2]);
    assert!((volume(&state) - (PLATE - 2.0 * HOLE)).abs() < 1e-6);
}

#[test]
fn an_allowed_reorder_changes_the_timeline_and_undoes() {
    let mut s = Session::new();
    s.plate_with_hole();
    // A second hole, sketched on the origin plane, may swap with the first.
    s.exec("sketch.create", json!({"plane": "xy", "name": "Second"}));
    s.exec(
        "sketch.add_point",
        json!({"sketch": 2, "at": {"x": 20, "y": 0}}),
    );
    s.exec(
        "feature.hole",
        json!({"sketch": 2, "diameter": 6, "depth": "through"}),
    );
    let state = s.exec("feature.move", json!({"feature": 2, "position": 1}));
    assert_eq!(order(&state), [0, 2, 1]);
    let state = s.undo();
    assert_eq!(order(&state), [0, 1, 2]);
}

#[test]
fn an_unknown_feature_is_refused() {
    let mut s = Session::new();
    s.plate_with_hole();
    let message = s.refused("feature.suppress", json!({"feature": 9}));
    assert!(!message.is_empty());
    let message = s.refused("feature.move", json!({"feature": 9, "position": 0}));
    assert!(!message.is_empty());
}
