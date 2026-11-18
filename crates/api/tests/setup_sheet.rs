// SPDX-License-Identifier: AGPL-3.0-or-later
//! The setup sheet: the printable page for the operator, made from a block
//! with a pocket and two setups.

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

/// A 60 x 40 x 30 block with a 20 x 10 pocket, a stock, a top setup with
/// everything detection finds, and a bottom setup turned by hand.
fn block() -> Session {
    let mut s = Session { api: Api::new() };
    s.exec("document.rename", json!({"name": "Sheet block"}));
    s.exec("sketch.create", json!({"plane": "xy", "name": "Outline"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 30}));
    s.exec("sketch.create", json!({"plane": "xy", "name": "Pocket"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 1, "from": {"x": 20, "y": 15}, "to": {"x": 40, "y": 25}}),
    );
    s.exec(
        "feature.extrude",
        json!({"sketch": 1, "distance": 8, "operation": "cut"}),
    );
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    s.exec("cam.add_setup", json!({"side": "top", "work_offset": 1}));
    let detected = s.call(json!({"method": "detect", "setup": 0}));
    let operations = detected["result"]["operations"].clone();
    assert!(!operations.as_array().unwrap().is_empty(), "{detected}");
    s.exec(
        "cam.add_operations",
        json!({"setup": 0, "operations": operations}),
    );
    s.exec("cam.add_setup", json!({"side": "bottom", "work_offset": 2}));
    s.exec("cam.add_operation", json!({"setup": 1, "type": "face"}));
    s
}

#[test]
fn the_sheet_lists_what_the_operator_needs() {
    let mut s = block();
    let path = std::env::temp_dir().join(format!("cndt-sheet-{}.html", std::process::id()));
    let reply = s.call(json!({
        "method": "setup_sheet",
        "machine": "mazak-hcn-5000",
        "path": path,
    }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["setups"], 2, "{reply}");
    let html = reply["result"]["html"].as_str().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), html);
    let _ = std::fs::remove_file(&path);

    // Self-contained and marked.
    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(html.contains("<style>"), "{html}");
    assert!(
        !html.contains("<link") && !html.contains("<script"),
        "{html}"
    );
    assert!(html.contains("SETUP SHEET <em>UNVERIFIED</em>"), "{html}");
    assert!(html.contains("Sheet block"), "{html}");
    // The machine's own post was chosen; the machine is named, unverified.
    assert!(html.contains("mazak-smoothg"), "{html}");
    assert!(html.contains("Mazak HCN-5000"), "{html}");
    assert!(html.contains("machine definition not verified"), "{html}");
    // Both setups, their offsets, how they are reached, zero and the stock:
    // 60 + 2 x 2 by 40 + 2 x 2 by 30 + 2 + 3.
    assert_eq!(html.matches("<section class=\"setup\">").count(), 2);
    assert!(html.contains("G54 work offset"), "{html}");
    assert!(html.contains("G55 work offset"), "{html}");
    assert!(html.contains("turned by hand"), "{html}");
    assert!(html.contains("turned over about X"), "{html}");
    assert!(html.contains("Where zero is"), "{html}");
    assert!(html.contains("64 x 44 x 35 mm"), "{html}");
    assert!(
        html.contains("allowances: sides 2, top 2, bottom 3 mm"),
        "{html}"
    );
    // Tools with their cutting data, typical until measured, and the time.
    assert!(html.contains("<th>Feed mm/min</th>"), "{html}");
    assert!(html.contains("<b class=\"typical\">TYPICAL</b>"), "{html}");
    assert!(html.contains("Estimated cutting time: <b>"), "{html}");
    // Every verification finding is on the page.
    let verify = s.call(json!({
        "method": "verify", "post": "mazak-smoothg", "machine": "mazak-hcn-5000"
    }));
    let report = &verify["result"];
    for finding in report["problems"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report["review"].as_array().unwrap())
    {
        let text = finding.as_str().unwrap().replace('&', "&amp;");
        let text = text
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        assert!(html.contains(&text), "missing finding {text:?}");
    }
    // Signing lines at the bottom.
    assert!(html.contains("Air cut done"), "{html}");
}

#[test]
fn the_same_project_gives_the_same_sheet_and_nothing_is_dated() {
    let mut s = block();
    let ask = |s: &mut Session| {
        let reply = s.call(json!({"method": "setup_sheet", "machine": "mazak-hcn-5000"}));
        reply["result"]["html"].as_str().unwrap().to_owned()
    };
    let first = ask(&mut s);
    let second = ask(&mut s);
    assert_eq!(first, second);
    // A fresh session builds the very same bytes.
    let mut other = block();
    assert_eq!(first, ask(&mut other));
    for year in ["2025", "2026", "2027"] {
        assert!(!first.contains(year), "a date-like {year} in the sheet");
    }
}

#[test]
fn one_setup_alone_and_clear_refusals() {
    let mut s = block();
    let reply = s.call(json!({"method": "setup_sheet", "setups": [1], "post": "grbl"}));
    assert_eq!(reply["ok"], true, "{reply}");
    let html = reply["result"]["html"].as_str().unwrap();
    assert_eq!(html.matches("<section class=\"setup\">").count(), 1);
    assert!(html.contains("G55 work offset"), "{html}");
    assert!(!html.contains("G54 work offset"), "{html}");
    assert!(
        html.contains("none chosen: machine limits were not checked"),
        "{html}"
    );

    let reply = s.call(json!({"method": "setup_sheet", "post": "no-such-post"}));
    assert_eq!(reply["ok"], false, "{reply}");
    let reply = s.call(json!({"method": "setup_sheet", "machine": "no-such-machine"}));
    assert_eq!(reply["error"]["kind"], "unknown_machine", "{reply}");
    let reply = s.call(json!({"method": "setup_sheet", "setups": []}));
    assert_eq!(reply["ok"], false, "{reply}");
    let reply = s.call(json!({"method": "setup_sheet", "setups": [9]}));
    assert_eq!(reply["ok"], false, "{reply}");
    let reply = s.call(json!({"method": "setup_sheet", "bogus": 1}));
    assert_eq!(reply["error"]["kind"], "bad_request", "{reply}");
    // No setups at all.
    let mut empty = Session { api: Api::new() };
    let reply = empty.call(json!({"method": "setup_sheet"}));
    assert_eq!(reply["ok"], false, "{reply}");
}
