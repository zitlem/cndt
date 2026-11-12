// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests: run the real `cndt` binary.

// This whole file is test code; the no-panic rule is for library code.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Command;

fn cndt(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cndt"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn info_describes_the_box() {
    let (ok, stdout, stderr) = cndt(&["info", &fixture("box-10x20x30.step")]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("solids    1"), "{stdout}");
    assert!(stdout.contains("faces     6"), "{stdout}");
    assert!(
        stdout.contains("size      10.000 x 20.000 x 30.000 mm"),
        "{stdout}"
    );
    assert!(stderr.is_empty(), "the kernel must not print: {stderr}");
}

#[test]
fn info_on_a_missing_file_fails_cleanly() {
    let (ok, stdout, stderr) = cndt(&["info", &fixture("missing.step")]);
    assert!(!ok);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error: cannot read"), "{stderr}");
}

#[test]
fn version_names_the_kernel() {
    let (ok, stdout, _) = cndt(&["version"]);
    assert!(ok);
    assert!(stdout.contains("OpenCascade 7"), "{stdout}");
}
