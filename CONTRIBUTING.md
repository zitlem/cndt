# Contributing to CNDT

Thank you for helping. A few rules here are strict because breaking them would put the whole project at risk.

## Licensing of contributions

- CNDT is licensed under AGPL-3.0-or-later. Your contribution is licensed the same way.
- You keep the copyright on what you write. There is no contributor license agreement and there never will be.
- Every commit must be signed off under the [Developer Certificate of Origin](https://developercertificate.org/). Use `git commit -s`, which adds a `Signed-off-by` line. By signing off you confirm you have the right to submit the work.

## Independent implementation

CNDT must be written without using other vendors' protected work. Do not contribute if any of these apply to your change:

- You looked at the source code of a commercial CAD/CAM product to write it.
- You decompiled, disassembled or traced a commercial product to learn how it works.
- You copied icons, images, help text, tutorials, sample files or exact dialog layouts from another product.
- It reads or writes an undocumented proprietary file format.

If you have worked on a commercial CAD/CAM product's internals, please do not contribute to the equivalent area of CNDT.

Good sources are published papers, textbooks, public standards, vendor user manuals read as a user, and open-source code under a compatible license.

## Record your sources

When you implement an algorithm, add its source to [docs/provenance.md](docs/provenance.md) in the same change. If you adapted open-source code, name the project, the license and the commit or version.

## Dependencies

New dependencies must be compatible with AGPL-3.0. MIT, Apache-2.0, BSD, Boost, LGPL and GPL-3.0 are acceptable. GPL-2.0-only and proprietary licenses are not. Explain why the dependency is needed in your change.

`cargo xtask licenses` checks every package in the dependency tree against the allow list in `xtask/src/main.rs`, and CI runs it on every change.

## Architecture

Read [docs/architecture.md](docs/architecture.md) first. Changes that break a rule there will not be merged. A change that needs a rule to move starts with a decision record in [docs/decisions/](docs/decisions/README.md).

## Safety

Read [docs/safety.md](docs/safety.md). In short:

- Anything that produces G-code needs a golden-file test.
- A change that alters an existing golden file must explain every difference.
- Never loosen a verification check to make a test pass.
- Operations and posts are marked verified only after a real cut on a real machine.

## Tests

- Geometry, solver and toolpath code needs unit or property tests.
- All tests must pass on Windows, macOS and Linux.
- Output must be deterministic: the same inputs give the same bytes on every platform.

## Posts and machine definitions

Posts and machine definitions for specific controllers are welcome. State in the change whether the output has been run on a real machine, on which controller, and with what result.
