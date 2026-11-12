// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 1 boundary to the B-rep geometry kernel.
//!
//! This is the only crate allowed to use OpenCascade. Every other crate uses
//! the trait and types defined here, so the kernel can be replaced later
//! (decision 0007, architecture rule 3).
