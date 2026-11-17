// SPDX-License-Identifier: AGPL-3.0-or-later
//! A setup on the part's right side, turned to by the B table (decision
//! 0019): a block 60 × 40 × 30 with a hole drilled into its right face.

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
}

fn check_golden(name: &str, output: &str) {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.nc"));
    if std::env::var_os("CNDT_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, output).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        output == expected,
        "{name}: output differs from {}; if intended, regenerate and review.\n--- got ---\n{output}",
        path.display()
    );
}

/// A frame vector (an array) or point (an object with x, y and z).
fn vec3(v: &Value) -> [f64; 3] {
    if v.is_object() {
        return ["x", "y", "z"].map(|k| v[k].as_f64().unwrap());
    }
    [0, 1, 2].map(|i| v[i].as_f64().unwrap())
}

#[test]
fn a_hole_in_the_right_side_on_the_b_table() {
    let mut s = Session { api: Api::new() };
    s.exec("document.rename", json!({"name": "Side block"}));
    s.exec("sketch.create", json!({"plane": "xy", "name": "Outline"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    let state = s.exec("feature.extrude", json!({"sketch": 0, "distance": 30}));

    // The right face: the side face whose sketch frame looks along +X.
    let faces: Vec<String> = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .filter(|name| name.starts_with("f0/side/"))
        .map(str::to_owned)
        .collect();
    let mut found = None;
    for face in faces {
        let state = s.exec("sketch.create", json!({"face": face}));
        let id = state["document"]["next_sketch"].as_u64().unwrap() - 1;
        let frame = &state["sketch_frames"][id.to_string()]["frame"];
        if (vec3(&frame["normal"])[0] - 1.0).abs() < 1e-9 {
            found = Some((id, frame.clone()));
            break;
        }
        s.exec("sketch.delete", json!({"sketch": id}));
    }
    let (sketch, frame) = found.expect("a face looking along +X");
    // A point at model (60, 20, 15), in the face sketch's coordinates.
    let (o, x, y) = (vec3(&frame["origin"]), vec3(&frame["x"]), vec3(&frame["y"]));
    let d = [60.0 - o[0], 20.0 - o[1], 15.0 - o[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    s.exec(
        "sketch.add_point",
        json!({"sketch": sketch, "at": {"x": dot(d, x), "y": dot(d, y)}}),
    );
    let state = s.exec(
        "feature.hole",
        json!({"sketch": sketch, "diameter": 6.6, "depth": 12, "name": "Side hole"}),
    );
    assert_eq!(state["feature_status"]["1"]["state"], "ok", "{state}");

    // A pocket from the top, 20 × 10 × 8: its left wall faces +X, towards
    // the right side, but the part covers it from there.
    let top = s.exec("sketch.create", json!({"plane": "xy", "name": "Pocket"}));
    let pocket = top["document"]["next_sketch"].as_u64().unwrap() - 1;
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": pocket, "from": {"x": 20, "y": 15}, "to": {"x": 40, "y": 25}}),
    );
    let state = s.exec(
        "feature.extrude",
        json!({"sketch": pocket, "distance": 8, "operation": "cut"}),
    );
    assert_eq!(state["feature_status"]["2"]["state"], "ok", "{state}");
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    let state = s.exec(
        "cam.add_setup",
        json!({"side": "right", "index": 90, "work_offset": 2}),
    );
    assert_eq!(
        state["document"]["cam"]["setups"][0]["setup"]["name"],
        "Setup 1 (right)"
    );
    // Detection from the right offers the hole, but no pocket: the wall
    // of the top pocket faces this side but is covered by the part.
    let detected = s.call(json!({"method": "detect", "setup": 0}));
    let operations = detected["result"]["operations"].as_array().unwrap();
    assert!(
        operations.iter().any(|o| o["type"] == "holes"),
        "{detected}"
    );
    assert!(
        operations.iter().all(|o| o["type"] != "pocket"),
        "{detected}"
    );
    assert!(
        detected["result"]["notes"].to_string().contains("covered"),
        "{detected}"
    );
    s.exec("cam.add_operation", json!({"setup": 0, "type": "holes"}));

    // Posted on the HCN-5000: the table turns to B90, then the hole is
    // drilled at program (15, 20): the part turned about Y puts model
    // (60, 20, 15) at (−15, 20, 60), and the work offset is the turned
    // part's front-left corner, (−30, 0, 60).
    let reply = s.call(
        json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": 0}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(gcode.contains("G0 B90."), "{gcode}");
    assert!(gcode.contains("G55"), "{gcode}");
    let drills: Vec<&str> = gcode
        .lines()
        .filter(|l| l.contains(" G81 ") || l.contains(" G83 "))
        .collect();
    assert!(!drills.is_empty(), "{gcode}");
    assert!(drills.iter().all(|l| l.contains("X15. Y20.")), "{drills:?}");
    let notes = reply["result"]["notes"].to_string();
    assert!(notes.contains("which way B turns"), "{notes}");
    assert!(notes.contains("whole raw block"), "{notes}");
    check_golden("side-block-right-b90-mazak-smoothg", gcode);

    // Simulated on the turned part: the hole is cut, and no rapid hits.
    let sim = s.call(json!({"method": "simulate", "setup": 0, "cell": 0.5}));
    assert_eq!(sim["ok"], true, "{sim}");
    // The one finding: the spot drill's cone breaks the mouth of a hole
    // the model leaves sharp, as it would on the top side. A review item.
    for gouge in sim["result"]["gouges"].as_array().unwrap() {
        assert_eq!(gouge["operation"], "Spot drill", "{sim}");
        assert!(gouge["depth"].as_f64().unwrap() < 0.5, "{sim}");
    }
    assert_eq!(sim["result"]["collisions"], json!([]), "{sim}");
    let verify = s.call(json!({"method": "verify", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setups": [0]}));
    assert_ne!(verify["result"]["verdict"], "problems", "{verify}");
    // Verification reads the posted G-code back and simulates it too.
    let text = verify["result"]["text"].as_str().unwrap();
    assert!(
        text.contains("Posted G-code read back and simulated: it cuts what was planned"),
        "{text}"
    );

    // A post with a bug, X and Y swapped in its drilling cycle: the plan
    // and its simulation are fine, but the G-code drills somewhere else,
    // and verification says so as a problem.
    let shipped =
        s.call(json!({"method": "post_source", "post": "mazak-smoothg"}))["result"]["text"]
            .as_str()
            .unwrap()
            .to_owned();
    let buggy = shipped.replace("drill = \"G81 X{x} Y{y}", "drill = \"G81 X{y} Y{x}");
    assert_ne!(buggy, shipped);
    let verify = s.call(json!({"method": "verify", "definition": buggy, "machine": "mazak-hcn-5000", "setups": [0]}));
    assert_eq!(verify["result"]["verdict"], "problems", "{verify}");
    assert!(
        verify["result"]["problems"]
            .to_string()
            .contains("does not cut what was planned"),
        "{verify}"
    );

    // Drawn in the model's own coordinates: the drill goes into the right
    // face at y 20, z 15, from x 60 inwards.
    let path = s.api.toolpath_preview(Some(0)).unwrap();
    let count = u32::from_le_bytes(path[8..12].try_into().unwrap()) as usize;
    let mut deepest = f64::INFINITY;
    for k in 0..count {
        let at = 12 + k * 25;
        let f = |i: usize| {
            f64::from(f32::from_le_bytes(
                path[at + 1 + 4 * i..at + 5 + 4 * i].try_into().unwrap(),
            ))
        };
        let (to_x, to_y, to_z) = (f(3), f(4), f(5));
        if path[at] == 1 && (to_y - 20.0).abs() < 1e-3 && (to_z - 15.0).abs() < 1e-3 {
            deepest = deepest.min(to_x);
        }
    }
    // 12 deep to the full diameter, plus the 118° point: 3.3 / tan 59°.
    let point = 3.3 / 59f64.to_radians().tan();
    assert!(
        (deepest - (60.0 - 12.0 - point)).abs() < 1e-3,
        "deepest cut at model x {deepest}"
    );
}
