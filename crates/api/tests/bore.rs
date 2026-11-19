// SPDX-License-Identifier: AGPL-3.0-or-later
//! Holes larger than every drill are bored with an end mill by helical
//! interpolation (decision 0030).

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

/// A 60 × 40 × 20 plate with one through hole of `diameter` at (30, 20).
fn plate(diameter: f64) -> Session {
    let mut s = Session { api: Api::new() };
    s.exec("document.rename", json!({"name": "Bored plate"}));
    s.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    let state = s.exec("feature.extrude", json!({"sketch": 0, "distance": 20}));
    let top = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|name| name.starts_with("f0/end/"))
        .unwrap()
        .to_owned();
    let state = s.exec("sketch.create", json!({"face": top}));
    let sketch = state["document"]["next_sketch"].as_u64().unwrap() - 1;
    let frame = &state["sketch_frames"][sketch.to_string()]["frame"];
    let o = &frame["origin"];
    let (dx, dy) = (
        30.0 - o["x"].as_f64().unwrap(),
        20.0 - o["y"].as_f64().unwrap(),
    );
    let ax = |k: &str, i: usize| frame[k][i].as_f64().unwrap();
    s.exec(
        "sketch.add_point",
        json!({"sketch": sketch, "at": {"x": dx * ax("x", 0) + dy * ax("x", 1), "y": dx * ax("y", 0) + dy * ax("y", 1)}}),
    );
    s.exec(
        "feature.hole",
        json!({"sketch": sketch, "diameter": diameter, "depth": "through", "name": "Big hole"}),
    );
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    s.exec("cam.add_setup", json!({"side": "top"}));
    s.exec("cam.add_operation", json!({"setup": 0, "type": "holes"}));
    s
}

#[test]
fn a_hole_larger_than_every_drill_is_bored_with_an_end_mill() {
    let mut s = plate(18.0);
    let reply = s.call(
        json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": 0}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    // The 10 mm end mill (the largest that leaves wall stock and is at least
    // half the hole) circles at radius 4: from X34. Y20., about (30, 20).
    assert!(gcode.contains("(BORE 18 MM X 21 MM)"), "{gcode}");
    assert!(gcode.contains("G3 X34. Y20."), "{gcode}");
    assert!(gcode.contains("I-4. J0."), "{gcode}");
    // Through: 1 mm past the part's 20 mm bottom.
    assert!(gcode.contains("Z-21."), "{gcode}");
    check_golden("bored-plate-mazak-smoothg", gcode);

    // The simulation: the hole is cut to size, nothing hit or gouged, and
    // the posted G-code cuts what was planned.
    let verify = s.call(json!({"method": "verify", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setups": [0]}));
    let result = &verify["result"];
    assert_ne!(result["verdict"], "problems", "{verify}");
    let text = result["text"].as_str().unwrap();
    assert!(text.contains("it cuts what was planned"), "{text}");
    for item in result["review"].as_array().unwrap() {
        let item = item.as_str().unwrap();
        assert!(!item.contains("below the model"), "no gouge: {item}");
    }
}

#[test]
fn a_hole_no_end_mill_can_bore_says_which_tool_is_needed() {
    let mut s = plate(30.0);
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "setup": 0}));
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("end mill between 15 and 29.8 mm to bore the 30 mm holes"),
        "{reply}"
    );
}
