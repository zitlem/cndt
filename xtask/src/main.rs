// SPDX-License-Identifier: AGPL-3.0-or-later
//! Project automation. Run with `cargo xtask <command>`:
//!
//! - `ci`: everything CI runs: format, lints, tests, layers, headers,
//!   licenses.
//! - `layers`: check that every crate only depends on lower layers.
//! - `headers`: check that every Rust file starts with the license header.
//! - `licenses`: check that every dependency's license is AGPL-compatible.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// First line of every Rust source file.
const HEADER: &str = "// SPDX-License-Identifier: AGPL-3.0-or-later";

/// Every crate under `crates/` and `apps/`, with its layer from
/// `docs/architecture.md`. A crate may only depend on strictly lower layers.
/// Layer 6 is the TypeScript frontend, which has no crate.
const LAYERS: &[(&str, u8)] = &[
    ("cndt-geom", 0),
    ("cndt-occt-sys", 0),
    ("cndt-kernel", 1),
    ("cndt-sketch", 1),
    ("cndt-model", 2),
    ("cndt-cam-core", 2),
    ("cndt-process-mill", 3),
    ("cndt-process-turn", 3),
    ("cndt-process-wedm", 3),
    ("cndt-sim", 3),
    ("cndt-post", 3),
    ("cndt-format", 3),
    ("cndt-engine", 4),
    ("cndt-api", 5),
    ("cndt-mcp", 5),
    ("cndt-cli", 7),
    ("cndt-desktop", 7),
    ("cndt-web", 7),
];

/// Crates that only the named workspace crates may depend on.
const RESTRICTED: &[(&str, &[&str])] = &[
    // The kernel is the only door to OpenCascade (architecture rule 3).
    ("cndt-occt-sys", &["cndt-kernel"]),
    // Nothing below the desktop app may know about the UI shell (rule 4).
    ("tauri", &["cndt-desktop"]),
];

/// Licenses that may be combined into an AGPL-3.0-or-later program
/// (decision 0001). Anything else fails `cargo xtask licenses` and needs a
/// deliberate decision before it is added here.
const ALLOWED_LICENSES: &[&str] = &[
    "AGPL-3.0-or-later",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC0-1.0",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "ISC",
    "LGPL-2.1",
    "LGPL-2.1-only",
    "LGPL-2.1-or-later",
    "MIT",
    "MPL-2.0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Unlicense",
    "Zlib",
];

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("ci") => ci(),
        Some("layers") => root().and_then(|root| layers(&root)),
        Some("headers") => root().and_then(|root| headers(&root)),
        Some("licenses") => root().and_then(|root| licenses(&root)),
        _ => Err(String::from(
            "usage: cargo xtask <ci|layers|headers|licenses>",
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("cannot find the workspace root"))
}

fn ci() -> Result<(), String> {
    let root = root()?;
    // The desktop app embeds the built frontend, so build it first. This
    // also type-checks every TypeScript file.
    let ui = root.join("ui");
    if !ui.join("node_modules").is_dir() {
        npm(&ui, &["ci", "--no-fund", "--no-audit"])?;
    }
    npm(&ui, &["run", "build"])?;
    cargo(&root, &["fmt", "--all", "--check"])?;
    cargo(
        &root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    cargo(&root, &["test", "--workspace", "--locked"])?;
    layers(&root)?;
    headers(&root)?;
    licenses(&root)?;
    println!("ci: all checks passed");
    Ok(())
}

fn cargo(root: &Path, args: &[&str]) -> Result<(), String> {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("> cargo {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

fn npm(dir: &Path, args: &[&str]) -> Result<(), String> {
    // On Windows npm is a batch file, which must be named with its extension.
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };
    println!("> npm {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|error| format!("could not run npm (is Node.js installed?): {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`npm {}` failed", args.join(" ")))
    }
}

struct Manifest {
    name: String,
    dependencies: Vec<String>,
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS
        .iter()
        .find(|(crate_name, _)| *crate_name == name)
        .map(|(_, layer)| *layer)
}

fn layers(root: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    for path in manifests(root)? {
        let manifest = parse_manifest(&path)?;
        let Some(own) = layer_of(&manifest.name) else {
            problems.push(format!(
                "{} is not in the layer map. Add it to LAYERS in xtask/src/main.rs and to docs/architecture.md.",
                manifest.name
            ));
            continue;
        };
        for dependency in &manifest.dependencies {
            if dependency.starts_with("cndt-") {
                match layer_of(dependency) {
                    None => problems.push(format!(
                        "{} depends on {dependency}, which is not in the layer map.",
                        manifest.name
                    )),
                    Some(theirs) if theirs >= own => problems.push(format!(
                        "{} (layer {own}) may not depend on {dependency} (layer {theirs}). Dependencies must point to a lower layer.",
                        manifest.name
                    )),
                    Some(_) => {}
                }
            }
            for (restricted, allowed) in RESTRICTED {
                let matches =
                    dependency == restricted || dependency.starts_with(&format!("{restricted}-"));
                if matches && !allowed.contains(&manifest.name.as_str()) {
                    problems.push(format!(
                        "{} may not depend on {dependency}; only {} may.",
                        manifest.name,
                        allowed.join(", ")
                    ));
                }
            }
        }
    }
    report("layers", &problems)
}

fn manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    for group in ["crates", "apps"] {
        let dir = root.join(group);
        if !dir.is_dir() {
            continue;
        }
        let entries = fs::read_dir(&dir)
            .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        for entry in entries {
            let manifest = entry
                .map_err(|error| error.to_string())?
                .path()
                .join("Cargo.toml");
            if manifest.is_file() {
                found.push(manifest);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Reads a crate's name and the names of everything it depends on. The project
/// convention is one dependency per line, written inline, so a line scan is
/// enough; table-style dependency sections are rejected.
fn parse_manifest(path: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut section = String::new();
    let mut name = None;
    let mut dependencies = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = line.to_owned();
            if section.contains("dependencies.") {
                return Err(format!(
                    "{}: write dependencies inline, one per line, not as `{section}` tables",
                    path.display()
                ));
            }
            continue;
        }
        if section == "[package]" && name.is_none() {
            if let Some((key, value)) = line.split_once('=')
                && key.trim() == "name"
            {
                name = Some(value.trim().trim_matches('"').to_owned());
            }
        } else if section.ends_with("dependencies]") {
            let key = line
                .split(['=', '.', ' '])
                .next()
                .unwrap_or_default()
                .trim();
            if !key.is_empty() {
                dependencies.push(key.to_owned());
            }
        }
    }
    let name = name.ok_or_else(|| format!("{}: no package name", path.display()))?;
    Ok(Manifest { name, dependencies })
}

fn headers(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    for group in ["crates", "apps", "xtask"] {
        collect_rust_files(&root.join(group), &mut files)?;
    }
    files.sort();
    let mut problems = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        if text.lines().next() != Some(HEADER) {
            problems.push(format!("{} must start with `{HEADER}`", file.display()));
        }
    }
    report("headers", &problems)
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries =
        fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .is_some_and(|name| name == "target" || name == "node_modules");
            if !skip {
                collect_rust_files(&path, out)?;
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn report(check: &str, problems: &[String]) -> Result<(), String> {
    if problems.is_empty() {
        println!("{check}: ok");
        Ok(())
    } else {
        for problem in problems {
            eprintln!("  {problem}");
        }
        Err(format!("{check}: {} problem(s)", problems.len()))
    }
}

/// Checks the license of every package in the full dependency tree,
/// including platform-specific ones, against [`ALLOWED_LICENSES`].
fn licenses(root: &Path) -> Result<(), String> {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(program)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("could not run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cannot parse cargo metadata: {error}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| String::from("cargo metadata has no package list"))?;

    let mut problems = Vec::new();
    for package in packages {
        let name = package["name"].as_str().unwrap_or("?");
        let version = package["version"].as_str().unwrap_or("?");
        match package["license"].as_str() {
            None => problems.push(format!(
                "{name} {version} declares no SPDX license; review it by hand"
            )),
            Some(expression) => match license_allowed(expression) {
                Ok(true) => {}
                Ok(false) => problems.push(format!(
                    "{name} {version} is licensed {expression:?}, which is not on the allow list"
                )),
                Err(reason) => problems.push(format!(
                    "{name} {version}: cannot read license {expression:?}: {reason}"
                )),
            },
        }
    }
    problems.sort();
    problems.dedup();
    report(
        &format!("licenses ({} packages)", packages.len()),
        &problems,
    )
}

/// Evaluates an SPDX expression: `OR` needs one allowed side, `AND` needs
/// both. `A WITH exception` is checked as a whole. Old-style `A/B` means
/// `A OR B`.
fn license_allowed(expression: &str) -> Result<bool, String> {
    let spaced = expression
        .replace('/', " OR ")
        .replace('(', " ( ")
        .replace(')', " ) ");
    let mut tokens: Vec<String> = Vec::new();
    let mut words = spaced.split_whitespace();
    while let Some(word) = words.next() {
        if word == "WITH" {
            let exception = words.next().ok_or("WITH without an exception")?;
            let license = tokens.pop().ok_or("WITH without a license")?;
            tokens.push(format!("{license} WITH {exception}"));
        } else {
            tokens.push(word.to_owned());
        }
    }
    let mut position = 0;
    let allowed = parse_or(&tokens, &mut position)?;
    if position == tokens.len() {
        Ok(allowed)
    } else {
        Err(format!("unexpected {:?}", tokens.get(position)))
    }
}

fn parse_or(tokens: &[String], position: &mut usize) -> Result<bool, String> {
    let mut allowed = parse_and(tokens, position)?;
    while tokens.get(*position).is_some_and(|token| token == "OR") {
        *position += 1;
        let right = parse_and(tokens, position)?;
        allowed = allowed || right;
    }
    Ok(allowed)
}

fn parse_and(tokens: &[String], position: &mut usize) -> Result<bool, String> {
    let mut allowed = parse_term(tokens, position)?;
    while tokens.get(*position).is_some_and(|token| token == "AND") {
        *position += 1;
        let right = parse_term(tokens, position)?;
        allowed = allowed && right;
    }
    Ok(allowed)
}

fn parse_term(tokens: &[String], position: &mut usize) -> Result<bool, String> {
    let token = tokens
        .get(*position)
        .ok_or_else(|| String::from("expression ends early"))?;
    *position += 1;
    if token == "(" {
        let allowed = parse_or(tokens, position)?;
        if tokens.get(*position).is_some_and(|token| token == ")") {
            *position += 1;
            Ok(allowed)
        } else {
            Err(String::from("missing )"))
        }
    } else if token == ")" || token == "AND" || token == "OR" {
        Err(format!("unexpected {token}"))
    } else {
        Ok(ALLOWED_LICENSES.contains(&token.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::license_allowed;

    #[test]
    fn license_expressions() {
        let cases = [
            ("MIT", true),
            ("MIT OR Apache-2.0", true),
            ("GPL-2.0-only", false),
            ("GPL-2.0-only OR MIT", true),
            ("(MIT OR Apache-2.0) AND Unicode-3.0", true),
            ("MIT AND SSPL-1.0", false),
            ("Apache-2.0 WITH LLVM-exception OR MIT", true),
            ("MIT/Apache-2.0", true),
            ("Proprietary", false),
        ];
        for (expression, expected) in cases {
            assert_eq!(license_allowed(expression), Ok(expected), "{expression}");
        }
        assert!(license_allowed("(MIT").is_err());
        assert!(license_allowed("MIT OR").is_err());
    }
}
