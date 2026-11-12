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

fn temp_dir(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cndt-cli-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn create_edit_and_show_a_project() {
    let dir = temp_dir("project");
    let file = dir.join("bracket.cndt").display().to_string();

    let (ok, stdout, stderr) = cndt(&["new", &file, "Bracket"]);
    assert!(ok, "{stderr}");
    assert!(stdout.starts_with("created"), "{stdout}");

    let (ok, stdout, stderr) = cndt(&[
        "exec",
        &file,
        "parameter.set",
        r#"{"name":"thickness","value":12,"unit":"mm","comment":"plate"}"#,
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.starts_with("Set thickness: saved"), "{stdout}");

    let (ok, stdout, _) = cndt(&["show", &file]);
    assert!(ok);
    assert_eq!(
        stdout,
        "Bracket\nparameters:\n  thickness = 12.0 mm  # plate\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_failed_command_leaves_the_file_untouched() {
    let dir = temp_dir("failed");
    let file = dir.join("part.cndt");
    let name = file.display().to_string();
    assert!(cndt(&["new", &name]).0);
    let before = std::fs::read(&file).unwrap();

    let (ok, _, stderr) = cndt(&["exec", &name, "parameter.remove", r#"{"name":"nope"}"#]);
    assert!(!ok);
    assert!(stderr.contains("no parameter named nope"), "{stderr}");
    let (ok, _, stderr) = cndt(&["exec", &name, "parameter.set", "{not json"]);
    assert!(!ok);
    assert!(stderr.contains("not valid JSON"), "{stderr}");

    assert_eq!(std::fs::read(&file).unwrap(), before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn new_refuses_to_overwrite() {
    let dir = temp_dir("overwrite");
    let file = dir.join("part.cndt").display().to_string();
    assert!(cndt(&["new", &file]).0);
    let (ok, _, stderr) = cndt(&["new", &file]);
    assert!(!ok);
    assert!(stderr.contains("already exists"), "{stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn api_mode_answers_each_line() {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_cndt"))
        .arg("api")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = concat!(
        r#"{"method":"execute","command":"parameter.set","args":{"name":"w","value":5,"unit":"mm"}}"#,
        "\n\n",
        r#"{"method":"undo"}"#,
        "\n",
        r#"{"method":"oops"}"#,
        "\n"
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["result"]["undo"], "Set w");
    assert_eq!(lines[1]["result"]["dirty"], false);
    assert_eq!(lines[2]["error"]["kind"], "bad_request");
}
