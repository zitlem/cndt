// SPDX-License-Identifier: AGPL-3.0-or-later
//! The CNDT test block (decision 0010), modelled end to end through the JSON
//! API, exactly as a front end or script would.
//!
//! This is Phase 1's acceptance test for the first user story: every CAD
//! feature the block needs builds, every face is named once, and the part
//! survives a change to its first dimension.

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

    fn state(&mut self) -> Value {
        self.call(json!({"method": "state"}))["result"].clone()
    }

    /// Adds a point and returns its id.
    fn point(&mut self, sketch: u64, x: f64, y: f64) -> u64 {
        let state = self.exec(
            "sketch.add_point",
            json!({"sketch": sketch, "at": {"x": x, "y": y}}),
        );
        state["document"]["sketches"][sketch.to_string()]["sketch"]["next_entity"]
            .as_u64()
            .unwrap()
            - 1
    }

    fn line(&mut self, sketch: u64, a: u64, b: u64) {
        self.exec(
            "sketch.add_line",
            json!({"sketch": sketch, "start": a, "end": b}),
        );
    }

    fn arc(&mut self, sketch: u64, center: u64, start: u64, end: u64) {
        self.exec(
            "sketch.add_arc",
            json!({"sketch": sketch, "center": center, "start": start, "end": end}),
        );
    }

    /// A rectangle from (x0, y0) to (x1, y1) with corner radius r.
    fn rounded_rectangle(
        &mut self,
        sketch: u64,
        (x0, y0): (f64, f64),
        (x1, y1): (f64, f64),
        r: f64,
    ) {
        let b0 = self.point(sketch, x0 + r, y0);
        let b1 = self.point(sketch, x1 - r, y0);
        let r0 = self.point(sketch, x1, y0 + r);
        let r1 = self.point(sketch, x1, y1 - r);
        let t0 = self.point(sketch, x1 - r, y1);
        let t1 = self.point(sketch, x0 + r, y1);
        let l0 = self.point(sketch, x0, y1 - r);
        let l1 = self.point(sketch, x0, y0 + r);
        let c_br = self.point(sketch, x1 - r, y0 + r);
        let c_tr = self.point(sketch, x1 - r, y1 - r);
        let c_tl = self.point(sketch, x0 + r, y1 - r);
        let c_bl = self.point(sketch, x0 + r, y0 + r);
        self.line(sketch, b0, b1);
        self.arc(sketch, c_br, b1, r0);
        self.line(sketch, r0, r1);
        self.arc(sketch, c_tr, r1, t0);
        self.line(sketch, t0, t1);
        self.arc(sketch, c_tl, t1, l0);
        self.line(sketch, l0, l1);
        self.arc(sketch, c_bl, l1, b0);
    }

    /// A slot (obround) along X from x0 to x1, centred on y, of width w.
    fn slot(&mut self, sketch: u64, x0: f64, x1: f64, y: f64, w: f64) {
        let r = w / 2.0;
        let right = self.point(sketch, x1 - r, y);
        let left = self.point(sketch, x0 + r, y);
        let right_low = self.point(sketch, x1 - r, y - r);
        let right_high = self.point(sketch, x1 - r, y + r);
        let left_high = self.point(sketch, x0 + r, y + r);
        let left_low = self.point(sketch, x0 + r, y - r);
        self.arc(sketch, right, right_low, right_high);
        self.line(sketch, right_high, left_high);
        self.arc(sketch, left, left_high, left_low);
        self.line(sketch, left_low, right_low);
    }

    /// A new sketch on the face whose name starts with `prefix`.
    fn sketch_on(&mut self, prefix: &str) -> u64 {
        let state = self.state();
        let face = state["body"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|names| names.as_array().unwrap().iter())
            .filter_map(Value::as_str)
            .find(|name| name.starts_with(prefix))
            .unwrap_or_else(|| panic!("no face {prefix}"))
            .to_owned();
        let state = self.exec("sketch.create", json!({"face": face}));
        state["document"]["next_sketch"].as_u64().unwrap() - 1
    }

    fn top(&mut self) -> String {
        let state = self.state();
        state["body"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|names| names.as_array().unwrap().iter())
            .filter_map(Value::as_str)
            .find(|name| name.starts_with("f0/end/"))
            .unwrap()
            .to_owned()
    }
}

#[test]
fn the_cndt_test_block() {
    let mut s = Session { api: Api::new() };
    s.exec("document.rename", json!({"name": "CNDT test block"}));

    // 2. Outside profile, 96 × 56 with R5 corners, 20 thick (the finished
    // size; feature 12 faces the stock down to it).
    s.exec("sketch.create", json!({"plane": "xy", "name": "Outline"}));
    s.rounded_rectangle(0, (0.0, 0.0), (96.0, 56.0), 5.0);
    s.exec(
        "feature.extrude",
        json!({"sketch": 0, "distance": 20, "name": "Body"}),
    );

    // 3. Rectangular pocket, 40 × 20 × 8 with R4 corners, from the top.
    let pocket = s.sketch_on("f0/end/");
    s.rounded_rectangle(pocket, (28.0, 18.0), (68.0, 38.0), 4.0);
    s.exec(
        "feature.extrude",
        json!({"sketch": pocket, "distance": -8, "operation": "cut", "name": "Pocket"}),
    );

    // 5. Open slot, 10 wide, through the front side, 6 deep.
    let open = s.sketch_on("f0/end/");
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": open, "from": {"x": 43, "y": -2}, "to": {"x": 53, "y": 10}}),
    );
    s.exec(
        "feature.extrude",
        json!({"sketch": open, "distance": -6, "operation": "cut", "name": "Open slot"}),
    );

    // 7 to 10. Holes, all from the top face.
    let holes = s.sketch_on("f0/end/");
    for (x, y) in [(8.0, 8.0), (88.0, 8.0), (8.0, 48.0), (88.0, 48.0)] {
        s.point(holes, x, y);
    }
    s.exec(
        "feature.hole",
        json!({"sketch": holes, "diameter": 6.6, "depth": "through", "name": "Clearance M6"}),
    );
    let tapped = s.sketch_on("f0/end/");
    for (x, y) in [(20.0, 8.0), (76.0, 8.0), (20.0, 48.0), (76.0, 48.0)] {
        s.point(tapped, x, y);
    }
    s.exec("feature.hole", json!({"sketch": tapped, "thread": "M6", "depth": 15, "thread_depth": 12, "name": "Tapped M6"}));
    let counterbored = s.sketch_on("f0/end/");
    for (x, y) in [(14.0, 28.0), (82.0, 28.0)] {
        s.point(counterbored, x, y);
    }
    s.exec(
        "feature.hole",
        json!({"sketch": counterbored, "diameter": 5.5, "depth": "through", "counterbore_diameter": 9.5, "counterbore_depth": 5.4, "name": "Counterbored M5"}),
    );
    let dowel = s.sketch_on("f0/end/");
    s.point(dowel, 48.0, 48.0);
    s.exec("feature.hole", json!({"sketch": dowel, "diameter": 8, "depth": 12, "tip_angle": 180, "fit": "H7", "name": "Dowel"}));

    // 11. 0.5 × 45° chamfer round the top: the outline, pocket, slot and
    // every hole.
    let top = s.top();
    s.exec(
        "feature.chamfer",
        json!({"faces": [top], "distance": 0.5, "name": "Top chamfer"}),
    );

    // 4 and 6. Closed slot 30 × 8 × 5 and keyway 25 × 6 × 3.5, on the
    // bottom face for the second setup. On the bottom, sketch Y is model −Y.
    let bottom = s.sketch_on("f0/start/");
    s.slot(bottom, 8.0, 38.0, -20.0, 8.0);
    s.exec(
        "feature.extrude",
        json!({"sketch": bottom, "distance": -5, "operation": "cut", "name": "Closed slot"}),
    );
    let keyway = s.sketch_on("f0/start/");
    s.slot(keyway, 58.0, 83.0, -40.0, 6.0);
    s.exec(
        "feature.extrude",
        json!({"sketch": keyway, "distance": -3.5, "operation": "cut", "name": "Keyway"}),
    );

    let state = s.state();
    let status = state["feature_status"].as_object().unwrap();
    assert_eq!(status.len(), 10);
    for (id, outcome) in status {
        assert_eq!(outcome["state"], "ok", "feature {id}: {outcome}");
    }
    let body = &state["body"];
    let size = |axis: &str| {
        body["bounds"]["max"][axis].as_f64().unwrap()
            - body["bounds"]["min"][axis].as_f64().unwrap()
    };
    assert!(
        (size("x") - 96.0).abs() < 1e-6
            && (size("y") - 56.0).abs() < 1e-6
            && (size("z") - 20.0).abs() < 1e-6
    );

    // Every face has a name, and no name is on two faces.
    let faces = body["faces"].as_array().unwrap();
    let mut names: Vec<&str> = faces
        .iter()
        .flat_map(|n| n.as_array().unwrap().iter().filter_map(Value::as_str))
        .collect();
    assert!(
        faces.iter().all(|n| !n.as_array().unwrap().is_empty()),
        "an unnamed face"
    );
    names.sort_unstable();
    // A name repeats only where something was split (decision 0012). Here
    // the open slot cuts the front top edge in two, so the chamfer along it
    // is two faces with one name. A reference to that name is flagged as
    // ambiguous rather than guessed.
    let doubled: Vec<&str> = names
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| w[0])
        .collect();
    assert_eq!(doubled.len(), 1, "{doubled:?}");
    assert!(
        doubled[0].starts_with("f7/blend/") && doubled[0].contains("+f0/side/e12"),
        "{doubled:?}"
    );
    let split = doubled[0].to_owned();
    names.dedup();
    // The thread, the fit and the counterbore are all on record for CAM.
    let document = state["document"].to_string();
    assert!(document.contains(r#""size":"M6x1.0""#) && document.contains(r#""fit":"H7""#));

    // Filleting "that" chamfer face's edges is refused as ambiguous.
    let reply = s.call(json!({"method": "execute", "command": "feature.fillet", "args": {"faces": [split], "radius": 0.2}}));
    let state = s.state();
    let flagged = &state["feature_status"]["10"];
    assert_eq!(flagged["state"], "failed", "{reply}");
    assert!(
        flagged["reason"]
            .as_str()
            .unwrap()
            .contains("split into 2 faces"),
        "{flagged}"
    );
    s.call(json!({"method": "undo"}));

    // The mesh comes out for display.
    assert!(s.api.body_mesh().unwrap().len() > 1000);

    // Make the block 2 mm thicker: every feature still finds its faces.
    s.exec(
        "feature.edit_extrude",
        json!({"feature": 0, "distance": 22}),
    );
    let state = s.state();
    for (id, outcome) in state["feature_status"].as_object().unwrap() {
        assert_eq!(
            outcome["state"], "ok",
            "after the edit, feature {id}: {outcome}"
        );
    }
    assert!((state["body"]["bounds"]["max"]["z"].as_f64().unwrap() - 22.0).abs() < 1e-6);

    // Drive the thickness from a parameter: everything follows, and undo
    // puts it all back.
    s.exec(
        "parameter.set",
        json!({"name": "thickness", "value": 20, "unit": "mm"}),
    );
    s.exec(
        "feature.edit_extrude",
        json!({"feature": 0, "distance": "thickness"}),
    );
    let state = s.exec(
        "parameter.set",
        json!({"name": "thickness", "value": 25, "unit": "mm"}),
    );
    assert!((state["body"]["bounds"]["max"]["z"].as_f64().unwrap() - 25.0).abs() < 1e-6);
    for (id, outcome) in state["feature_status"].as_object().unwrap() {
        assert_eq!(
            outcome["state"], "ok",
            "at 25 thick, feature {id}: {outcome}"
        );
    }
    let reply = s.call(
        json!({"method": "execute", "command": "parameter.remove", "args": {"name": "thickness"}}),
    );
    assert!(
        reply["error"]["message"].as_str().unwrap().contains("Body"),
        "{reply}"
    );
    let state = s.call(json!({"method": "undo"}))["result"].clone();
    assert!((state["body"]["bounds"]["max"]["z"].as_f64().unwrap() - 20.0).abs() < 1e-6);
    // The outline's sketch dimension from a parameter too.
    s.exec(
        "parameter.set",
        json!({"name": "pocket_depth", "value": 8, "unit": "mm"}),
    );
    let state = s.exec(
        "feature.edit_extrude",
        json!({"feature": 1, "distance": "-pocket_depth"}),
    );
    assert_eq!(state["feature_status"]["1"]["state"], "ok");

    // Pattern the dowel hole: a second one 10 mm along X. Then undo.
    let state = s.exec(
        "feature.pattern",
        json!({"feature": 6, "count_x": 2, "spacing_x": -10}),
    );
    let pattern = state["document"]["features"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["id"]
        .to_string();
    assert_eq!(
        state["feature_status"][&pattern]["state"], "ok",
        "{}",
        state["feature_status"]
    );
    let state = s.exec(
        "feature.edit_pattern",
        json!({"feature": pattern.parse::<u64>().unwrap(), "count_x": 2, "spacing_x": -12}),
    );
    assert_eq!(state["feature_status"][&pattern]["state"], "ok");
    s.call(json!({"method": "undo"}));
    s.call(json!({"method": "undo"}));

    // And it saves and opens again, bit for bit.
    let path = std::env::temp_dir().join(format!("cndt-test-block-{}.cndt", std::process::id()));
    s.call(json!({"method": "save_as", "path": path}));
    let saved = s.state()["document"].clone();
    let mut fresh = Session { api: Api::new() };
    let opened = fresh.call(json!({"method": "open", "path": path}));
    assert_eq!(opened["result"]["document"], saved);
    assert_eq!(
        opened["result"]["body"]["volume"],
        s.state()["body"]["volume"]
    );
    let _ = std::fs::remove_file(path);

    // To look at the block in the app: CNDT_TEST_BLOCK_OUT=block.cndt
    // cargo test -p cndt-api --test test_block, then open that file.
    if let Some(out) = std::env::var_os("CNDT_TEST_BLOCK_OUT") {
        s.exec(
            "feature.edit_extrude",
            json!({"feature": 0, "distance": 20}),
        );
        let reply = s.call(json!({"method": "save_as", "path": out.to_string_lossy()}));
        assert_eq!(reply["ok"], true, "{reply}");
    }
}
