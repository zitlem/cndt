// SPDX-License-Identifier: AGPL-3.0-or-later
//! A slot as wide as an end mill is milled along its centreline in one
//! width (decision 0031), not pocketed with a smaller cutter.

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

/// A 60 × 40 × 20 block with a slot `width` wide and 5 deep on its top,
/// its ends' centres at (15, 20) and (45, 20); one setup with a pocket on
/// the slot's floor (and a finishing allowance if given).
fn block(width: f64, finish: Option<f64>) -> Session {
    let mut s = Session { api: Api::new() };
    s.exec("document.rename", json!({"name": "Slotted block"}));
    s.exec("sketch.create", json!({"plane": "xy", "name": "Block"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 20}));
    // On the XY plane, offset up to the top: a sketch at the origin plane
    // cut upward through the top would not do; sketch on the top face.
    let state = s.call(json!({"method": "state"}))["result"].clone();
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
    let local = |x: f64, y: f64| {
        let (dx, dy) = (x - o["x"].as_f64().unwrap(), y - o["y"].as_f64().unwrap());
        let ax = |k: &str, i: usize| frame[k][i].as_f64().unwrap();
        json!({"x": dx * ax("x", 0) + dy * ax("x", 1), "y": dx * ax("y", 0) + dy * ax("y", 1)})
    };
    s.exec(
        "sketch.add_slot",
        json!({"sketch": sketch, "from": local(15.0, 20.0), "to": local(45.0, 20.0), "width": width}),
    );
    let state = s.exec(
        "feature.extrude",
        json!({"sketch": sketch, "distance": -5, "operation": "cut", "name": "Slot"}),
    );
    let floor = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|name| name.starts_with("f1/end/"))
        .unwrap()
        .to_owned();
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    s.exec("cam.add_setup", json!({"side": "top"}));
    s.exec("cam.add_operation", json!({"setup": 0, "type": "face"}));
    let mut pocket = json!({"setup": 0, "type": "pocket", "floor": floor});
    if let Some(finish) = finish {
        pocket["finish"] = json!(finish);
    }
    s.exec("cam.add_operation", pocket);
    s
}

#[test]
fn a_slot_as_wide_as_an_end_mill_is_milled_along_its_centreline() {
    let mut s = block(6.0, None);
    let reply = s.call(
        json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": 0}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    // The 6 mm end mill (T12), along y 20 from x 15 to x 45.
    assert!(gcode.contains("(SLOT 6 MM)"), "{gcode}");
    assert!(gcode.contains("(T12 6 MM 3 FLUTE END MILL)"), "{gcode}");
    assert!(gcode.contains("G0 X45. Y20."), "{gcode}");
    assert!(gcode.contains("X15. Z-5."), "{gcode}");
    check_golden("slotted-block-mazak-smoothg", gcode);

    // Cut to size, nothing hit or gouged, and the G-code cuts the plan.
    let verify = s.call(json!({"method": "verify", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setups": [0]}));
    let result = &verify["result"];
    assert_ne!(result["verdict"], "problems", "{verify}");
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("it cuts what was planned"),
        "{verify}"
    );
    for item in result["review"].as_array().unwrap() {
        let item = item.as_str().unwrap();
        assert!(!item.contains("below the model"), "no gouge: {item}");
        assert!(!item.contains("left on the part"), "cut to size: {item}");
    }
}

#[test]
fn a_finishing_allowance_or_no_matching_cutter_pockets_it_instead() {
    // With a finishing allowance the slot is pocketed and finished.
    let mut s = block(6.0, Some(0.2));
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "setup": 0}));
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(!gcode.contains("(SLOT "), "{gcode}");
    assert!(
        gcode.contains("(POCKET 5 MM DEEP, 0.2 MM FINISH)"),
        "{gcode}"
    );
    // An 8 mm slot: no 8 mm end mill, so it is pocketed with a smaller one.
    let mut s = block(8.0, None);
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "setup": 0}));
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(!gcode.contains("(SLOT "), "{gcode}");
}
