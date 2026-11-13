// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cndt`: the headless command-line runner.
//!
//! Everything the app can do must be possible from here (architecture rule 4).
//! Every command goes through the same JSON API as the other front ends.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

use cndt_api::Api;
use serde_json::{Value, json};

const USAGE: &str = "\
usage: cndt <command>

project files:
  new <file.cndt> [name]                    create a new project file
  show <file.cndt>                          print a project's name and parameters
  exec <file.cndt> <command> [json-args]    run a command on a project and save it
  commands                                  list the commands exec accepts

other:
  post <file.cndt> [post] [setup]           print G-code for a setup (post: fanuc-mill or grbl; setup id)
  info <file.step>                          show the topology and size of a STEP file
  api                                       answer JSON requests on stdin, one per line
  version                                   show the CNDT and geometry kernel versions

example:
  cndt new bracket.cndt Bracket
  cndt exec bracket.cndt parameter.set '{\"name\":\"thickness\",\"value\":12,\"unit\":\"mm\"}'";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["new", path] => new(Path::new(path), None),
        ["new", path, name] => new(Path::new(path), Some(name)),
        ["show", path] => show(Path::new(path)),
        ["exec", path, command] => exec(Path::new(path), command, "{}"),
        ["exec", path, command, arguments] => exec(Path::new(path), command, arguments),
        ["commands"] => commands(),
        ["info", path] => info(Path::new(path)),
        ["post", path] => post(Path::new(path), "fanuc-mill", None),
        ["post", path, id] => post(Path::new(path), id, None),
        ["post", path, id, setup] => match setup.parse::<u32>() {
            Ok(setup) => post(Path::new(path), id, Some(setup)),
            Err(_) => Err(format!("{setup:?} is not a setup id")),
        },
        ["api"] => serve_stdin(),
        ["version" | "--version" | "-V"] => {
            println!("cndt {}", env!("CARGO_PKG_VERSION"));
            println!("{}", Api::new().kernel_description());
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Sends one request through the API and returns its result, or its error
/// message.
fn call(api: &mut Api, request: &Value) -> Result<Value, String> {
    let reply: Value = serde_json::from_str(&api.handle_json(&request.to_string()))
        .map_err(|error| format!("unreadable API reply: {error}"))?;
    if reply["ok"] == true {
        Ok(reply["result"].clone())
    } else {
        Err(reply["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
            .to_owned())
    }
}

fn open(api: &mut Api, path: &Path) -> Result<Value, String> {
    call(api, &json!({"method": "open", "path": path}))
}

fn new(path: &Path, name: Option<&str>) -> Result<(), String> {
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let name = name.map_or_else(
        || {
            path.file_stem().map_or_else(
                || String::from("Untitled"),
                |stem| stem.to_string_lossy().into_owned(),
            )
        },
        str::to_owned,
    );
    let mut api = Api::new();
    call(&mut api, &json!({"method": "new_document", "name": name}))?;
    call(&mut api, &json!({"method": "save_as", "path": path}))?;
    println!("created {}", path.display());
    Ok(())
}

/// Prints G-code to standard output and notes to standard error, so
/// `cndt post part.cndt > part.nc` gives a clean program file.
fn post(path: &Path, id: &str, setup: Option<u32>) -> Result<(), String> {
    let mut api = Api::new();
    open(&mut api, path)?;
    let mut request = json!({"method": "post", "post": id});
    if let Some(setup) = setup {
        request["setup"] = json!(setup);
    }
    let result = call(&mut api, &request)?;
    if let Some(notes) = result["notes"].as_array() {
        for note in notes {
            eprintln!("note: {}", note.as_str().unwrap_or_default());
        }
    }
    print!("{}", result["gcode"].as_str().unwrap_or_default());
    Ok(())
}

fn show(path: &Path) -> Result<(), String> {
    let mut api = Api::new();
    let state = open(&mut api, path)?;
    let document = &state["document"];
    println!("{}", document["name"].as_str().unwrap_or_default());
    let parameters = document["parameters"].as_object();
    match parameters {
        Some(parameters) if !parameters.is_empty() => {
            println!("parameters:");
            for (name, parameter) in parameters {
                let unit = match parameter["unit"].as_str() {
                    Some("millimetre") => " mm",
                    Some("degree") => " deg",
                    _ => "",
                };
                let comment = parameter["comment"]
                    .as_str()
                    .map(|comment| format!("  # {comment}"))
                    .unwrap_or_default();
                println!("  {name} = {}{unit}{comment}", parameter["value"]);
            }
        }
        _ => println!("no parameters"),
    }
    Ok(())
}

fn exec(path: &Path, command: &str, arguments: &str) -> Result<(), String> {
    let args: Value = serde_json::from_str(arguments)
        .map_err(|error| format!("the arguments are not valid JSON: {error}"))?;
    let mut api = Api::new();
    open(&mut api, path)?;
    let state = call(
        &mut api,
        &json!({"method": "execute", "command": command, "args": args}),
    )?;
    call(&mut api, &json!({"method": "save"}))?;
    let label = state["undo"].as_str().unwrap_or(command);
    println!("{label}: saved {}", path.display());
    Ok(())
}

fn commands() -> Result<(), String> {
    let mut api = Api::new();
    let specs = call(&mut api, &json!({"method": "commands"}))?;
    for spec in specs.as_array().into_iter().flatten() {
        println!(
            "{}  {}",
            spec["name"].as_str().unwrap_or_default(),
            spec["description"].as_str().unwrap_or_default()
        );
        for arg in spec["args"].as_array().into_iter().flatten() {
            let required = if arg["required"] == true {
                "required"
            } else {
                "optional"
            };
            println!(
                "    {} ({}, {required}): {}",
                arg["name"].as_str().unwrap_or_default(),
                arg["kind"].as_str().unwrap_or_default(),
                arg["description"].as_str().unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// Answers one JSON request per input line with one JSON response line,
/// until end of input. Blank lines are skipped.
fn serve_stdin() -> Result<(), String> {
    let mut api = Api::new();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|error| format!("cannot read input: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        writeln!(out, "{}", api.handle_json(&line))
            .and_then(|()| out.flush())
            .map_err(|error| format!("cannot write output: {error}"))?;
    }
    Ok(())
}

fn info(path: &Path) -> Result<(), String> {
    let mut api = Api::new();
    let summary = call(&mut api, &json!({"method": "step_info", "path": path}))?;
    println!("{}", path.display());
    for key in ["solids", "shells", "faces", "edges", "vertices"] {
        println!("  {key:<9} {}", summary[key]);
    }
    let bounds = &summary["bounds"];
    if bounds.is_object() {
        let coordinate =
            |corner: &str, axis: &str| bounds[corner][axis].as_f64().unwrap_or_default();
        let size = |axis: &str| coordinate("max", axis) - coordinate("min", axis);
        println!(
            "  size      {:.3} x {:.3} x {:.3} mm",
            size("x"),
            size("y"),
            size("z")
        );
        println!(
            "  min       {:.3}, {:.3}, {:.3} mm",
            coordinate("min", "x"),
            coordinate("min", "y"),
            coordinate("min", "z")
        );
    }
    Ok(())
}
