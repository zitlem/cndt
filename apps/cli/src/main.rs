// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cndt`: the headless command-line runner.
//!
//! Everything the app can do must be possible from here (architecture rule 4).
//! It calls the kernel directly for now; once the engine and API crates exist,
//! commands go through them like every other front end.

use std::path::Path;
use std::process::ExitCode;

use cndt_kernel::{Kernel, OpenCascade};

const USAGE: &str = "\
usage: cndt <command>

commands:
  info <file.step>   show the topology and size of a STEP file
  version            show the CNDT and geometry kernel versions";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let kernel = OpenCascade;
    match args.as_slice() {
        ["info", path] => info(&kernel, Path::new(path)),
        ["version" | "--version" | "-V"] => {
            println!("cndt {}", env!("CARGO_PKG_VERSION"));
            println!("{}", kernel.description());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn info(kernel: &dyn Kernel, path: &Path) -> ExitCode {
    match kernel.import_step(path) {
        Ok(summary) => {
            println!("{}", path.display());
            println!("  solids    {}", summary.solids);
            println!("  shells    {}", summary.shells);
            println!("  faces     {}", summary.faces);
            println!("  edges     {}", summary.edges);
            println!("  vertices  {}", summary.vertices);
            if let Some(bounds) = summary.bounds {
                let [x, y, z] = bounds.size();
                let min = bounds.min;
                println!("  size      {x:.3} x {y:.3} x {z:.3} mm");
                println!("  min       {:.3}, {:.3}, {:.3} mm", min.x, min.y, min.z);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
