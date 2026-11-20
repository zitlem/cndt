// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cancel and progress for long work (decision 0035): a simulation or a
//! verification that is cancelled from another thread fails with the kind
//! `cancelled` and changes nothing; the same request run again gives the
//! same result as one never cancelled.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cndt_api::Api;
use serde_json::{Value, json};

fn call(api: &mut Api, request: &Value) -> Value {
    serde_json::from_str(&api.handle_json(&request.to_string())).unwrap()
}

fn exec(api: &mut Api, command: &str, args: Value) -> Value {
    let reply = call(
        api,
        &json!({"method": "execute", "command": command, "args": args}),
    );
    assert_eq!(reply["ok"], true, "{command} {args}: {reply}");
    reply["result"].clone()
}

fn pocketed_block() -> Api {
    let mut api = Api::new();
    exec(
        &mut api,
        "sketch.create",
        json!({"plane": "xy", "name": "Outline"}),
    );
    exec(
        &mut api,
        "sketch.add_rectangle",
        json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 60, "y": 40}}),
    );
    let state = exec(
        &mut api,
        "feature.extrude",
        json!({"sketch": 0, "distance": 30}),
    );
    let bottom = state["body"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|names| names.as_array().unwrap().iter())
        .filter_map(Value::as_str)
        .find(|name| name.starts_with("f0/start/"))
        .unwrap()
        .to_owned();
    exec(
        &mut api,
        "cam.set_stock",
        json!({"side": 2, "top": 2, "bottom": 3}),
    );
    exec(&mut api, "cam.add_setup", json!({"side": "top"}));
    exec(
        &mut api,
        "cam.add_operation",
        json!({"setup": 0, "type": "contour", "face": bottom}),
    );
    api
}

/// Cancels in a loop until told to stop: a cancel that arrives while the
/// request runs, whatever the timing.
fn cancel_repeatedly(api: &Api) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let control = api.job_control();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            control.cancel();
            std::thread::yield_now();
        }
    });
    (stop, thread)
}

#[test]
fn a_cancelled_simulation_fails_as_cancelled_and_changes_nothing() {
    let mut api = pocketed_block();
    let request = json!({"method": "simulate", "setup": 0, "cell": 0.1});
    let before = call(&mut api, &request);
    assert_eq!(before["ok"], true, "{before}");
    let (done, total) = api.job_control().progress();
    assert!(total > 0 && done == total, "{done} of {total}");

    let (stop, thread) = cancel_repeatedly(&api);
    let cancelled = call(&mut api, &request);
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    assert_eq!(cancelled["ok"], false, "{cancelled}");
    assert_eq!(cancelled["error"]["kind"], "cancelled", "{cancelled}");

    // Not poisoned: the next request runs and equals the first.
    let after = call(&mut api, &request);
    assert_eq!(after, before);
}

#[test]
fn a_cancelled_verification_gives_no_report() {
    let mut api = pocketed_block();
    let request = json!({"method": "verify", "post": "mazak-smoothg", "setups": [0]});
    let before = call(&mut api, &request);
    assert_eq!(before["ok"], true, "{before}");

    let (stop, thread) = cancel_repeatedly(&api);
    let cancelled = call(&mut api, &request);
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    assert_eq!(cancelled["ok"], false, "{cancelled}");
    assert_eq!(cancelled["error"]["kind"], "cancelled", "{cancelled}");

    assert_eq!(call(&mut api, &request), before);
}

#[test]
fn a_cancel_with_no_work_running_does_not_spoil_the_next_request() {
    let mut api = pocketed_block();
    api.job_control().cancel();
    let reply = call(&mut api, &json!({"method": "simulate", "setup": 0}));
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn a_cancelled_post_fails_as_cancelled_and_the_next_one_is_unchanged() {
    let mut api = pocketed_block();
    let request = json!({"method": "post", "post": "mazak-smoothg", "setup": 0});
    let before = call(&mut api, &request);
    assert_eq!(before["ok"], true, "{before}");

    let (stop, thread) = cancel_repeatedly(&api);
    let cancelled = call(&mut api, &request);
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    assert_eq!(cancelled["ok"], false, "{cancelled}");
    assert_eq!(cancelled["error"]["kind"], "cancelled", "{cancelled}");

    assert_eq!(call(&mut api, &request), before);
}
