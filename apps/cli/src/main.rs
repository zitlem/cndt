// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cndt`: the headless command-line runner.
//!
//! Everything the app can do must be possible from here (architecture rule 4).

fn main() {
    println!("cndt {}", env!("CARGO_PKG_VERSION"));
}
