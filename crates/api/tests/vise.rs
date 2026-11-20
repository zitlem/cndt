// SPDX-License-Identifier: AGPL-3.0-or-later
//! A vise holding the stock (decision 0020): a contour down the part's
//! walls hits the jaws when they grip too high, and clears them when they
//! grip only the stock below the part.

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

    fn fixture_problems(&mut self) -> Vec<String> {
        let verify = self.call(json!({"method": "verify", "post": "mazak-smoothg", "setups": [0]}));
        assert_eq!(verify["ok"], true, "{verify}");
        verify["result"]["problems"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|p| p.contains("vise"))
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn a_contour_down_the_walls_hits_jaws_that_grip_too_high() {
    let mut s = Session { api: Api::new() };
    s.exec("sketch.create", json!({"plane": "xy", "name": "Outline"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    let state = s.exec("feature.extrude", json!({"sketch": 0, "distance": 30}));
    let bottom = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|name| name.starts_with("f0/start/"))
        .unwrap()
        .to_owned();
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    s.exec("cam.add_setup", json!({"side": "top"}));
    s.exec(
        "cam.add_operation",
        json!({"setup": 0, "type": "contour", "face": bottom}),
    );

    // No vise: nothing to hit.
    assert!(s.fixture_problems().is_empty());

    // Jaws gripping 10 mm of the 33 mm stock reach 7 mm up the part's
    // walls; the contour goes all the way down, so it cuts into them.
    let state = s.exec("cam.set_vise", json!({"setup": 0, "grip": 10}));
    assert_eq!(
        state["document"]["cam"]["setups"][0]["setup"]["vise"],
        json!({"grip": 10.0, "jaw": 25.0})
    );
    let problems = s.fixture_problems();
    assert!(!problems.is_empty(), "the contour must hit the jaws");
    assert!(problems[0].contains("Contour"), "{problems:?}");

    // The setup sheet tells the operator where the jaws must grip.
    let sheet = s.call(json!({"method": "setup_sheet", "post": "mazak-smoothg", "setups": [0]}));
    let html = sheet["result"]["html"].as_str().unwrap();
    assert!(
        html.contains("jaws on the front and back, gripping 10 mm of the stock from its bottom up"),
        "{html}"
    );

    // Gripping only 2 mm, inside the 3 mm of stock below the part: clear.
    s.exec("cam.set_vise", json!({"setup": 0, "grip": 2}));
    assert!(
        s.fixture_problems().is_empty(),
        "{:?}",
        s.fixture_problems()
    );

    // Jaws along Y clamp the left and right instead, and grip high there too.
    s.exec(
        "cam.set_vise",
        json!({"setup": 0, "grip": 10, "along": "y"}),
    );
    assert!(!s.fixture_problems().is_empty());

    // Removed: no grip given.
    let state = s.exec("cam.set_vise", json!({"setup": 0}));
    assert!(
        state["document"]["cam"]["setups"][0]["setup"]
            .get("vise")
            .is_none()
    );
    assert!(s.fixture_problems().is_empty());

    // Refused values say why.
    let reply = s.call(
        json!({"method": "execute", "command": "cam.set_vise", "args": {"setup": 0, "grip": -1}}),
    );
    assert!(
        reply["error"]["message"].as_str().unwrap().contains("grip"),
        "{reply}"
    );
}

/// A plate with no stock under it, on parallels under its clamped faces: a
/// through hole in the middle is clear, one near the clamped edge drills
/// into the parallel.
#[test]
fn a_through_hole_over_a_parallel_hits_it() {
    let hit = |y: f64, body: Option<f64>| -> Vec<String> {
        let mut s = Session { api: Api::new() };
        s.exec("sketch.create", json!({"plane": "xy", "name": "Plate"}));
        s.exec(
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
        );
        let state = s.exec("feature.extrude", json!({"sketch": 0, "distance": 10}));
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
        // The face sketch's frame: find where model (30, y) is in it.
        let frame = &state["sketch_frames"][sketch.to_string()]["frame"];
        let o = &frame["origin"];
        let (ox, oy) = (o["x"].as_f64().unwrap(), o["y"].as_f64().unwrap());
        let (xx, xy) = (
            frame["x"][0].as_f64().unwrap(),
            frame["x"][1].as_f64().unwrap(),
        );
        let (yx, yy) = (
            frame["y"][0].as_f64().unwrap(),
            frame["y"][1].as_f64().unwrap(),
        );
        let (dx, dy) = (30.0 - ox, y - oy);
        s.exec(
            "sketch.add_point",
            json!({"sketch": sketch, "at": {"x": dx * xx + dy * xy, "y": dx * yx + dy * yy}}),
        );
        s.exec(
            "feature.hole",
            json!({"sketch": sketch, "diameter": 6.6, "depth": "through"}),
        );
        s.exec("cam.set_stock", json!({"side": 2, "top": 1, "bottom": 0}));
        s.exec("cam.add_setup", json!({"side": "top"}));
        s.exec("cam.add_operation", json!({"setup": 0, "type": "holes"}));
        let mut vise = json!({"setup": 0, "grip": 5});
        if let Some(body) = body {
            vise["body"] = json!(body);
        }
        s.exec("cam.set_vise", vise);
        s.fixture_problems()
    };
    assert!(
        hit(20.0, None).is_empty(),
        "the middle is over the gap between the parallels"
    );
    // A floor 0.5 mm under the stock: the through drill goes past the
    // plate's bottom and into it, in the middle too.
    let floor = hit(20.0, Some(0.5));
    assert!(
        floor.iter().any(|p| p.contains("floor")),
        "a through drill into the vise's floor is a problem: {floor:?}"
    );
    // A floor far below is not reached.
    assert!(hit(20.0, Some(30.0)).is_empty());
    let problems = hit(3.0, None);
    assert!(
        problems.iter().any(|p| p.contains("parallel")),
        "a hole 3 mm from the clamped edge drills into the parallel: {problems:?}"
    );
}
