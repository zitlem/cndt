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

/// Compares G-code with a reviewed golden file. Regenerate with
/// `CNDT_UPDATE_GOLDEN=1 cargo test -p cndt-api --test test_block`, then
/// review every changed line (docs/safety.md, gate 3).
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

    // 12. 0.5 × 45° round the bottom: outline, slot, keyway and the exits
    // of the through holes.
    let underside = s.state()["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|name| name.starts_with("f0/start/"))
        .unwrap()
        .to_owned();
    s.exec(
        "feature.chamfer",
        json!({"faces": [underside], "distance": 0.5, "name": "Bottom chamfer"}),
    );

    let state = s.state();
    let status = state["feature_status"].as_object().unwrap();
    assert_eq!(status.len(), 11);
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
    let last = state["document"]["features"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["id"]
        .to_string();
    let flagged = &state["feature_status"][&last];
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

    // Phase 2: the holes, as G-code (decisions 0013 and 0014). Without a
    // setup there is nothing to post, and it says so.
    let reply = s.call(json!({"method": "post", "post": "fanuc-mill"}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no setups"),
        "{reply}"
    );
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    let state = s.exec("cam.add_setup", json!({"side": "top"}));
    assert_eq!(
        state["document"]["cam"]["setups"][0]["setup"]["name"],
        "Setup 1 (top)"
    );
    s.exec("cam.add_operation", json!({"setup": 0, "type": "face"}));
    // The outside profile, followed down the walls to the part's bottom
    // face (the 3 mm left under it holds the part until setup 2).
    let underside = s.state()["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|name| name.starts_with("f0/start/"))
        .unwrap()
        .to_owned();
    s.exec(
        "cam.add_operation",
        json!({"setup": 0, "type": "contour", "face": underside}),
    );
    // The pocket (feature 1) and the open slot (feature 2), down to their
    // floors: the end caps of those cuts.
    let floor = |s: &mut Session, feature: u32| {
        s.state()["body"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|names| names.as_array().unwrap().iter())
            .filter_map(Value::as_str)
            .find(|name| name.starts_with(&format!("f{feature}/end/")))
            .unwrap()
            .to_owned()
    };
    let pocket_floor = floor(&mut s, 1);
    s.exec(
        "cam.add_operation",
        json!({"setup": 0, "type": "pocket", "floor": pocket_floor}),
    );
    let open_floor = floor(&mut s, 2);
    s.exec(
        "cam.add_operation",
        json!({"setup": 0, "type": "pocket", "floor": open_floor}),
    );
    s.exec("cam.add_operation", json!({"setup": 0, "type": "holes"}));
    // The 0.5 mm chamfers round the top: outline, pocket, slot, every hole.
    let top = s.top();
    s.exec(
        "cam.add_operation",
        json!({"setup": 0, "type": "chamfer", "face": top, "width": 0.5}),
    );
    // The bottom setup: the part flipped about X. Face it to thickness; no
    // holes face it yet, so the holes operation adds nothing.
    s.exec("cam.add_setup", json!({"side": "bottom", "work_offset": 2}));
    s.exec("cam.add_operation", json!({"setup": 1, "type": "face"}));
    // The closed slot (feature 8) and the keyway (feature 9), from below.
    // The keyway's P9 fit wants its walls finished: 0.2 mm left by the
    // roughing, taken off by one full-depth lap.
    let slot_floor = floor(&mut s, 8);
    s.exec(
        "cam.add_operation",
        json!({"setup": 1, "type": "pocket", "floor": slot_floor}),
    );
    let keyway_floor = floor(&mut s, 9);
    s.exec(
        "cam.add_operation",
        json!({"setup": 1, "type": "pocket", "floor": keyway_floor, "finish": 0.2}),
    );
    // The 0.5 mm chamfers round the bottom: outline, closed slot, keyway.
    s.exec(
        "cam.add_operation",
        json!({"setup": 1, "type": "chamfer", "face": underside, "width": 0.5}),
    );
    // Feature detection finds the same operations from the solid alone
    // (less the keyway's finishing allowance, which is a design choice).
    let stored = |s: &mut Session, setup: usize| -> Vec<Value> {
        s.state()["document"]["cam"]["setups"][setup]["setup"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                let mut op = o["operation"].clone();
                op.as_object_mut().unwrap().remove("finish");
                op
            })
            .collect()
    };
    for setup in [0_u32, 1] {
        // Detecting again on a programmed setup adds nothing.
        let reply = s.call(json!({"method": "detect", "setup": setup}));
        assert_eq!(reply["result"]["operations"], json!([]), "{reply}");
        // On an empty setup of the same side, it finds them all.
        let side = if setup == 0 { "top" } else { "bottom" };
        s.exec("cam.add_setup", json!({"side": side}));
        let reply = s.call(json!({"method": "detect", "setup": 2}));
        assert_eq!(reply["ok"], true, "{reply}");
        let mut expected = stored(&mut s, setup as usize);
        if setup == 0 {
            // Setup 1, earlier, contours already: the copy goes after it.
            expected.retain(|o| o["type"] != "contour");
        }
        assert_eq!(
            reply["result"]["operations"].as_array().unwrap(),
            &expected,
            "setup {setup}: {reply}"
        );
        assert_eq!(reply["result"]["notes"], json!([]), "{reply}");
        s.call(json!({"method": "undo"}));
    }
    // Without its contour, setup 1 gets exactly that suggested back.
    let contour = s.state()["document"]["cam"]["setups"][0]["setup"]["operations"][1].clone();
    assert_eq!(contour["operation"]["type"], "contour");
    s.exec(
        "cam.remove_operation",
        json!({"setup": 0, "operation": contour["id"]}),
    );
    let reply = s.call(json!({"method": "detect", "setup": 0}));
    assert_eq!(
        reply["result"]["operations"],
        json!([contour["operation"]]),
        "{reply}"
    );
    s.call(json!({"method": "undo"}));
    // One click: a new setup takes every suggestion as one undoable step.
    s.exec("cam.add_setup", json!({"side": "top", "work_offset": 3}));
    let found = s.call(json!({"method": "detect", "setup": 2}))["result"]["operations"].clone();
    let state = s.exec(
        "cam.add_operations",
        json!({"setup": 2, "operations": found}),
    );
    let ops = state["document"]["cam"]["setups"][2]["setup"]["operations"]
        .as_array()
        .unwrap()
        .len();
    // Setup 1 already contours the outside, so this one does not.
    assert_eq!(ops, 5);
    s.call(json!({"method": "undo"}));
    assert_eq!(
        s.state()["document"]["cam"]["setups"][2]["setup"]["operations"],
        json!([])
    );
    s.call(json!({"method": "undo"}));
    assert_eq!(
        s.state()["document"]["cam"]["setups"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let reply = s.call(json!({"method": "post", "post": "fanuc-mill", "setup": 1}));
    assert_eq!(reply["ok"], true, "{reply}");
    check_golden(
        "test-block-setup-2-fanuc-mill",
        reply["result"]["gcode"].as_str().unwrap(),
    );
    let reply = s.call(json!({"method": "post", "post": "fanuc-mill", "setup": 0}));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    let notes = reply["result"]["notes"].to_string();
    // The counterbores are milled now, not left as a note.
    assert!(!notes.contains("Counterbored M5"), "{notes}");
    assert!(gcode.contains("(COUNTERBORE 9.5 MM X 5.4 MM)"), "{gcode}");
    check_golden("test-block-setup-1-fanuc-mill", gcode);
    // The preview for the 3D view: cuts and rapids, in model coordinates.
    let preview = s.api.toolpath_preview(Some(0)).unwrap();
    assert_eq!(&preview[..8], b"CNDTTPH1");
    let count = u32::from_le_bytes(preview[8..12].try_into().unwrap()) as usize;
    assert_eq!(preview.len(), 12 + count * 25);
    assert!(count > 1000, "{count}");
    // Written to a file as well.
    let nc = std::env::temp_dir().join(format!("cndt-test-block-{}.nc", std::process::id()));
    let reply = s.call(json!({"method": "post", "post": "fanuc-mill", "setup": 0, "path": nc}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(std::fs::read_to_string(&nc).unwrap(), gcode);
    let _ = std::fs::remove_file(nc);
    // The other shipped posts with rigid tapping take the whole setup too.
    for post in ["haas-mill", "linuxcnc-mill"] {
        let reply = s.call(json!({"method": "post", "post": post, "setup": 0}));
        assert_eq!(reply["ok"], true, "{post}: {reply}");
    }
    let linuxcnc = s.call(json!({"method": "post", "post": "linuxcnc-mill", "setup": 0}));
    let taps = linuxcnc["result"]["gcode"]
        .as_str()
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("G33.1 "))
        .count();
    assert_eq!(taps, 4, "one synchronised move per M6 hole");
    // GRBL cannot rigid-tap, and says so.
    let reply = s.call(json!({"method": "post", "post": "grbl"}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rigid tapping"),
        "{reply}"
    );
    // The Mazak HCN-5000 (decision 0015): both setups fit its limits and
    // post with its own post.
    for (setup, golden) in [
        (0, "test-block-setup-1-mazak-smoothg"),
        (1, "test-block-setup-2-mazak-smoothg"),
    ] {
        let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": setup}));
        assert_eq!(reply["ok"], true, "{reply}");
        let notes = reply["result"]["notes"].to_string();
        assert!(notes.contains("definition is unverified"), "{notes}");
        let gcode = reply["result"]["gcode"].as_str().unwrap();
        assert_eq!(gcode.matches("G84.2 ").count(), usize::from(setup == 0));
        check_golden(golden, gcode);
    }
    // Another controller's codes are refused: the Fanuc post's M29 is a
    // robot request on a Mazak.
    let reply = s.call(
        json!({"method": "post", "post": "fanuc-mill", "machine": "mazak-hcn-5000", "setup": 0}),
    );
    assert_eq!(reply["error"]["kind"], "machine", "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("takes the mazak-smoothg post"),
        "{reply}"
    );

    // Both sides in one program, the part turned over by hand: the tool
    // goes home and the program stops before the second side, on G55.
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setups": [0, 1]}));
    assert_eq!(reply["ok"], true, "{reply}");
    let turned = reply["result"]["gcode"].as_str().unwrap();
    assert!(
        turned.contains("\n(TURN THE PART OVER, CLAMP IT, SET G55 AND RESTART: SETUP 2 BOTTOM)\n"),
        "{turned}"
    );
    assert_eq!(turned.lines().filter(|l| l.ends_with(" M0")).count(), 1);
    check_golden("test-block-both-sides-turned-mazak-smoothg", turned);

    // Both sides on the B table: the part stands on the pallet, so turning
    // it to its bottom is a turn about model Y. Each side has its own
    // offset, set at its B angle.
    let ops = |s: &mut Session, setup: usize| -> Value {
        Value::Array(
            s.state()["document"]["cam"]["setups"][setup]["setup"]["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["operation"].clone())
                .collect(),
        )
    };
    let (top_ops, bottom_ops) = (ops(&mut s, 0), ops(&mut s, 1));
    let state = s.exec(
        "cam.add_setup",
        json!({"side": "top", "work_offset": 3, "index": 0, "name": "B0 top"}),
    );
    let setups = state["document"]["cam"]["setups"].as_array().unwrap();
    let b0 = setups.last().unwrap()["id"].as_u64().unwrap();
    s.exec(
        "cam.add_operations",
        json!({"setup": b0, "operations": top_ops}),
    );
    let state = s.exec(
        "cam.add_setup",
        json!({"side": "bottom", "flip_about": "y", "work_offset": 4, "index": 180, "name": "B180 bottom"}),
    );
    let setups = state["document"]["cam"]["setups"].as_array().unwrap();
    let b180 = setups.last().unwrap()["id"].as_u64().unwrap();
    s.exec(
        "cam.add_operations",
        json!({"setup": b180, "operations": bottom_ops}),
    );
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setups": [b0, b180]}));
    assert_eq!(reply["ok"], true, "{reply}");
    let indexed = reply["result"]["gcode"].as_str().unwrap();
    let b_lines: Vec<&str> = indexed
        .lines()
        .filter(|l| l.starts_with('N') && l.contains(" B"))
        .map(|l| l.split_once(' ').unwrap().1)
        .collect();
    assert_eq!(b_lines, ["G0 B0.", "G0 B180."], "{indexed}");
    assert!(
        !indexed.lines().any(|l| l.ends_with(" M0")),
        "no stop on the table"
    );
    check_golden("test-block-both-sides-indexed-mazak-smoothg", indexed);
    // The B180 side alone turns the table there first.
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": b180}));
    let alone = reply["result"]["gcode"].as_str().unwrap();
    assert!(alone.contains(" G0 B180.\n"), "{reply}");
    // A post for a machine without a B table refuses.
    let reply = s.call(json!({"method": "post", "post": "fanuc-mill", "setups": [b0, b180]}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot turn a rotary axis (B)"),
        "{reply}"
    );
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "setups": [b0, b0]}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("listed twice"),
        "{reply}"
    );
    // A tool faster than the spindle gives no G-code.
    s.exec(
        "cam.set_tool",
        json!({"number": 12, "kind": "end_mill", "diameter": 6, "rpm": 15000, "feed": 900}),
    );
    let reply = s.call(
        json!({"method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": 0}),
    );
    assert_eq!(reply["error"]["kind"], "machine", "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("15000 rpm is faster than the Mazak HCN-5000 (SmoothG)'s 12000 rpm"),
        "{reply}"
    );
    assert!(reply["result"].is_null(), "{reply}");
    s.call(json!({"method": "undo"}));
    let reply = s.call(json!({"method": "post", "post": "grbl", "machine": "lathe"}));
    assert_eq!(reply["error"]["kind"], "unknown_machine", "{reply}");

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
