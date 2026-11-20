// SPDX-License-Identifier: AGPL-3.0-or-later
//! The post gates of docs/safety.md, through the JSON API: gate 4 (a program
//! with verification problems is refused unless overridden, and the override
//! is listed in the header), gate 2 (an unverified post and machine are
//! flagged) and the naming of an edited or custom post in the header.

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

    fn exec(&mut self, command: &str, args: Value) {
        let reply = self.call(json!({"method": "execute", "command": command, "args": args}));
        assert_eq!(reply["ok"], true, "{command} {args}: {reply}");
    }

    fn shipped(&mut self, post: &str) -> String {
        self.call(json!({"method": "post_source", "post": post}))["result"]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

/// A 60 × 40 × 20 block, faced on top in one setup.
fn block() -> Session {
    let mut s = Session { api: Api::new() };
    s.exec("sketch.create", json!({"plane": "xy", "name": "Block"}));
    s.exec(
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    s.exec("feature.extrude", json!({"sketch": 0, "distance": 20}));
    s.exec("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3}));
    s.exec("cam.add_setup", json!({"side": "top"}));
    s.exec("cam.add_operation", json!({"setup": 0, "type": "face"}));
    s
}

/// The shipped Mazak post with every cutting move written as a rapid: a
/// problem the read-back finds (it cuts what was planned, but as G0).
fn rapid_cuts(s: &mut Session) -> String {
    let shipped = s.shipped("mazak-smoothg");
    let edited = shipped.replacen("linear = \"G1\"", "linear = \"G0\"", 1);
    assert_ne!(edited, shipped, "the post has no linear = \"G1\"");
    edited
}

#[test]
fn a_problem_refuses_the_post_without_the_override() {
    let mut s = block();
    let definition = rapid_cuts(&mut s);
    let reply = s.call(json!({
        "method": "post", "definition": definition, "machine": "mazak-hcn-5000", "setup": 0
    }));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["kind"], "verification", "{reply}");
    let message = reply["error"]["message"].as_str().unwrap();
    assert!(message.contains("or post with override"), "{message}");
    assert!(message.contains("a cutting move"), "{message}");
}

#[test]
fn the_override_posts_and_is_listed_in_the_header() {
    let mut s = block();
    let definition = rapid_cuts(&mut s);
    let reply = s.call(json!({
        "method": "post", "definition": definition, "machine": "mazak-hcn-5000",
        "setup": 0, "override": true
    }));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    // Header comments come before the first numbered block.
    let head = gcode.split("\nN10 ").next().unwrap();
    assert!(
        head.contains("VERIFICATION OVERRIDDEN BY THE USER"),
        "{gcode}"
    );
    // The problem itself is listed under it.
    assert!(head.to_uppercase().contains("A CUTTING MOVE"), "{gcode}");
}

#[test]
fn a_clean_program_has_no_override_line() {
    let mut s = block();
    let reply = s.call(json!({"method": "post", "post": "mazak-smoothg", "setup": 0}));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(!gcode.contains("OVERRIDDEN"), "{gcode}");
}

#[test]
fn a_definition_the_post_cannot_load_is_refused_even_with_the_override() {
    let mut s = block();
    let shipped = s.shipped("mazak-smoothg");
    let broken = shipped.replacen("linear = \"G1\"", "linear = \"G1\"\nbogus_key = 1", 1);
    let reply = s.call(json!({
        "method": "post", "definition": broken, "setup": 0, "override": true
    }));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_ne!(reply["error"]["kind"], "verification", "{reply}");
}

#[test]
fn an_unverified_post_and_machine_are_flagged() {
    let mut s = block();
    let reply = s.call(json!({
        "method": "post", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setup": 0
    }));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(
        gcode.contains("POST MAZAK-SMOOTHG 0.1.0 UNVERIFIED"),
        "{gcode}"
    );
    assert!(gcode.contains("UNVERIFIED POST"), "{gcode}");
    let notes = reply["result"]["notes"].to_string();
    assert!(notes.contains("definition is unverified"), "{notes}");

    // The verification report lists them for review, without blocking.
    let verify = s.call(json!({
        "method": "verify", "post": "mazak-smoothg", "machine": "mazak-hcn-5000", "setups": [0]
    }));
    assert_ne!(verify["result"]["verdict"], "problems", "{verify}");
    let review = verify["result"]["review"].to_string();
    assert!(review.contains("unverified"), "{review}");
}

#[test]
fn an_edited_post_is_named_in_the_header() {
    let mut s = block();
    let shipped = s.shipped("mazak-smoothg");
    // Unchanged text is the shipped post: no note.
    let reply = s.call(json!({"method": "post", "definition": shipped, "setup": 0}));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(!gcode.contains("EDITED POST"), "{gcode}");
    assert!(!gcode.contains("CUSTOM POST"), "{gcode}");

    // Any change is named, with the post it came from.
    let edited = shipped.replacen("linear = \"G1\"", "linear = \"G01\"", 1);
    assert_ne!(edited, shipped);
    let reply = s.call(json!({"method": "post", "definition": edited, "setup": 0}));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(
        gcode.contains("EDITED POST: CHANGED FROM THE SHIPPED MAZAK-SMOOTHG"),
        "{gcode}"
    );
}

#[test]
fn a_custom_post_is_named_in_the_header() {
    let mut s = block();
    let shipped = s.shipped("mazak-smoothg");
    let custom = shipped.replacen("id = \"mazak-smoothg\"", "id = \"my-shop-post\"", 1);
    assert_ne!(custom, shipped, "the post has no id = \"mazak-smoothg\"");
    let reply = s.call(json!({"method": "post", "definition": custom, "setup": 0}));
    assert_eq!(reply["ok"], true, "{reply}");
    let gcode = reply["result"]["gcode"].as_str().unwrap();
    assert!(
        gcode.contains("CUSTOM POST MY-SHOP-POST: NOT SHIPPED WITH CNDT"),
        "{gcode}"
    );
}
