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
