// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finds OpenCascade, compiles the C++ shim and links the OpenCascade libraries.
//!
//! Where OpenCascade is looked for, in order:
//! 1. `OCCT_DIR`: a prefix holding `include/opencascade` and `lib`
//!    (on Windows, a conda-style `Library` prefix also works).
//! 2. Homebrew on macOS (`brew --prefix opencascade`).
//! 3. The Debian and Ubuntu system locations on Linux.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// OpenCascade libraries the shim calls into directly.
const CORE_LIBS: &[&str] = &[
    "TKernel",
    "TKMath",
    "TKG2d",
    "TKG3d",
    "TKGeomBase",
    "TKBRep",
    "TKTopAlgo",
    "TKMesh",
    "TKXSBase",
    "TKGeomAlgo",
    "TKPrim",
    "TKBO",
    "TKBool",
    "TKShHealing",
];

/// STEP support: one library from 7.8 on, four before.
const STEP_LIB_NEW: &str = "TKDESTEP";
const STEP_LIBS_OLD: &[&str] = &["TKSTEP", "TKSTEPBase", "TKSTEPAttr", "TKSTEP209"];

struct BuildError(String);

impl fmt::Debug for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

struct Occt {
    include: PathBuf,
    lib: PathBuf,
}

fn main() -> Result<(), BuildError> {
    println!("cargo:rerun-if-changed=cpp/occt_shim.cpp");
    println!("cargo:rerun-if-env-changed=OCCT_DIR");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let occt = find_occt(&target_os)?;

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("cpp/occt_shim.cpp")
        .include(&occt.include)
        .warnings(false)
        .compile("cndt_occt_shim");

    println!("cargo:rustc-link-search=native={}", occt.lib.display());
    for lib in CORE_LIBS {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    if has_library(&occt.lib, STEP_LIB_NEW) {
        println!("cargo:rustc-link-lib=dylib={STEP_LIB_NEW}");
    } else {
        for lib in STEP_LIBS_OLD {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
    if target_os != "windows" {
        // Let test and app binaries find the libraries without extra setup.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", occt.lib.display());
    }
    Ok(())
}

fn find_occt(target_os: &str) -> Result<Occt, BuildError> {
    if let Some(dir) = std::env::var_os("OCCT_DIR") {
        let dir = PathBuf::from(dir);
        for prefix in [dir.clone(), dir.join("Library")] {
            if let Some(found) = occt_at(&prefix) {
                return Ok(found);
            }
        }
        return Err(BuildError(format!(
            "OCCT_DIR is set to {}, but no include/opencascade and lib were found there",
            dir.display()
        )));
    }
    if target_os == "macos"
        && let Some(prefix) = brew_prefix()
        && let Some(found) = occt_at(&prefix)
    {
        return Ok(found);
    }
    if target_os == "linux" {
        let include = PathBuf::from("/usr/include/opencascade");
        for lib in [
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/usr/lib64",
            "/usr/lib",
        ] {
            let lib = PathBuf::from(lib);
            if include.is_dir() && has_library(&lib, "TKernel") {
                return Ok(Occt { include, lib });
            }
        }
    }
    Err(BuildError(String::from(
        "OpenCascade was not found. Install it (macOS: `brew install opencascade`) \
         or set OCCT_DIR to its install prefix. See README.md.",
    )))
}

fn occt_at(prefix: &Path) -> Option<Occt> {
    let include = prefix.join("include").join("opencascade");
    let lib = prefix.join("lib");
    (include.is_dir() && lib.is_dir()).then_some(Occt { include, lib })
}

fn brew_prefix() -> Option<PathBuf> {
    let output = Command::new("brew")
        .args(["--prefix", "opencascade"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(PathBuf::from(text.trim()))
}

fn has_library(dir: &Path, name: &str) -> bool {
    ["lib{}.dylib", "lib{}.so", "{}.lib"]
        .iter()
        .any(|pattern| dir.join(pattern.replace("{}", name)).exists())
}
