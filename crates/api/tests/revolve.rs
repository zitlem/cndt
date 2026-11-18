// SPDX-License-Identifier: AGPL-3.0-or-later
//! The revolve feature through the JSON API (decision 0021): a stepped shaft
//! turned from a half-section, edited, undone, and refused with its reason.

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

    fn polygon(&mut self, sketch: u64, points: &[(f64, f64)]) {
        let mut ids = Vec::new();
        for (x, y) in points {
            let state = self.exec(
                "sketch.add_point",
                json!({"sketch": sketch, "at": {"x": x, "y": y}}),
            );
            let next = state["document"]["sketches"][sketch.to_string()]["sketch"]["next_entity"]
                .as_u64()
                .unwrap();
            ids.push(next - 1);
        }
        for i in 0..ids.len() {
            self.exec(
                "sketch.add_line",
                json!({"sketch": sketch, "start": ids[i], "end": ids[(i + 1) % ids.len()]}),
            );
        }
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

#[test]
fn a_shaft_is_turned_edited_and_undone() {
    let mut s = Session { api: Api::new() };
    s.exec("sketch.create", json!({"plane": "xy", "name": "Section"}));
    s.polygon(
        0,
        &[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 20.0),
            (6.0, 20.0),
            (6.0, 30.0),
            (0.0, 30.0),
        ],
    );
    let state = s.exec("feature.revolve", json!({"sketch": 0}));
    assert_eq!(state["feature_status"]["0"]["state"], "ok", "{state}");
    assert_eq!(
        state["document"]["features"][0]["feature"]["type"],
        "revolve"
    );
    assert_eq!(state["document"]["features"][0]["feature"]["angle"], 360.0);
    let full = std::f64::consts::PI * (100.0 * 20.0 + 36.0 * 10.0);
    assert!((volume(&state) - full).abs() < 1e-3, "{}", volume(&state));
    // Every swept face is named by the feature and the sketch line behind it.
    let names = face_names(&state);
    assert_eq!(names.len(), 5, "{names:?}");
    assert!(names.iter().all(|n| n.starts_with("f0/side/")), "{names:?}");

    // Half a turn, by an expression of a parameter.
    s.exec(
        "parameter.set",
        json!({"name": "sweep", "value": 180, "unit": "deg"}),
    );
    let state = s.exec(
        "feature.edit_revolve",
        json!({"feature": 0, "angle": "sweep"}),
    );
    assert_eq!(state["feature_status"]["0"]["state"], "ok", "{state}");
    assert!(
        (volume(&state) - full / 2.0).abs() < 1e-3,
        "{}",
        volume(&state)
    );
    assert!(
        face_names(&state)
            .iter()
            .any(|n| n.starts_with("f0/start/"))
    );
    assert!(face_names(&state).iter().any(|n| n.starts_with("f0/end/")));
    // Changing the parameter turns the shaft further.
    let state = s.exec(
        "parameter.set",
        json!({"name": "sweep", "value": 90, "unit": "deg"}),
    );
    assert!(
        (volume(&state) - full / 4.0).abs() < 1e-3,
        "{}",
        volume(&state)
    );

    // Undo takes it back, one step at a time.
    let undo = |s: &mut Session| {
        let reply = s.call(json!({"method": "undo"}));
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"].clone()
    };
    let state = undo(&mut s); // the parameter back to 180
    assert!(
        (volume(&state) - full / 2.0).abs() < 1e-3,
        "{}",
        volume(&state)
    );
    let state = undo(&mut s); // the edit: a full turn again
    assert!((volume(&state) - full).abs() < 1e-3, "{}", volume(&state));
    undo(&mut s); // the parameter itself
    let state = undo(&mut s); // the revolve
    assert_eq!(
        state["document"]["features"].as_array().map_or(0, Vec::len),
        0
    );
}

#[test]
fn a_profile_across_the_axis_is_refused_with_the_reason_and_the_axis_can_be_x() {
    let mut s = Session { api: Api::new() };
    s.exec("sketch.create", json!({"plane": "xy"}));
    s.polygon(0, &[(-2.0, 0.0), (3.0, 0.0), (3.0, 10.0), (-2.0, 10.0)]);
    let state = s.exec("feature.revolve", json!({"sketch": 0}));
    let reason = state["feature_status"]["0"]["reason"].as_str().unwrap();
    assert!(reason.contains("crosses the axis"), "{reason}");
    // About the sketch's X axis the same rectangle lies on one side.
    let state = s.exec("feature.edit_revolve", json!({"feature": 0, "axis": "x"}));
    assert_eq!(state["feature_status"]["0"]["state"], "ok", "{state}");
    let expected = std::f64::consts::PI * 100.0 * 5.0;
    assert!(
        (volume(&state) - expected).abs() < 1e-3,
        "{}",
        volume(&state)
    );
}

#[test]
fn bad_arguments_say_what_is_wrong() {
    let mut s = Session { api: Api::new() };
    s.exec("sketch.create", json!({"plane": "xy"}));
    s.polygon(0, &[(1.0, 0.0), (4.0, 0.0), (4.0, 3.0), (1.0, 3.0)]);
    let ask = |s: &mut Session, args: Value| {
        s.call(json!({"method": "execute", "command": "feature.revolve", "args": args}))
    };
    let reply = ask(&mut s, json!({"sketch": 0, "axis": "z"}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("use x or y"),
        "{reply}"
    );
    let reply = ask(&mut s, json!({"sketch": 0, "axis": "x", "axis_line": 2}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not both"),
        "{reply}"
    );
    for angle in [0, 361, -90] {
        let reply = ask(&mut s, json!({"sketch": 0, "angle": angle}));
        assert_eq!(reply["ok"], false, "{angle}: {reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("angle"),
            "{reply}"
        );
    }
    // An axis line that is not a line of the sketch fails the build, with
    // the reason in the feature's status.
    let state = s.exec("feature.revolve", json!({"sketch": 0, "axis_line": 0}));
    let reason = state["feature_status"]["0"]["reason"].as_str().unwrap();
    assert!(reason.contains("not a line"), "{reason}");
    let reply = ask(&mut s, json!({"sketch": 9}));
    assert_eq!(reply["ok"], false, "{reply}");
}
