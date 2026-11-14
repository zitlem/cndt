// SPDX-License-Identifier: AGPL-3.0-or-later
//! The verification report (decision 0016, slice 2): every check CNDT can
//! make on the setups about to be posted, sorted into problems (do not run
//! the program) and things to review, as plain text to keep with the
//! G-code.

use std::fmt::Write as _;
use std::path::Path;

use cndt_cam_core::{Event, Step};
use serde::Serialize;

use crate::{Api, ApiError};

/// The outcome of a verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Something would crash, break a machine limit or cannot be posted.
    Problems,
    /// Nothing blocks the program, but some findings need a person's eye.
    Review,
    /// Nothing found. The program is still unverified until it has cut.
    Clear,
}

/// A verification report.
#[derive(Debug, Clone, Serialize)]
pub struct Verification {
    /// The outcome.
    pub verdict: Verdict,
    /// What must be fixed before running.
    pub problems: Vec<String>,
    /// What a person should look at.
    pub review: Vec<String>,
    /// The whole report as text.
    pub text: String,
}

fn mm(value: f64) -> String {
    format!("{value:.2} mm")
}

fn at(p: cndt_geom::Point3) -> String {
    format!("X{:.2} Y{:.2} Z{:.2}", p.x, p.y, p.z)
}

impl Api {
    /// Verifies the given setups (all of them, in order, if `None`) as one
    /// program through `post`, on `machine` if given: plans, machine
    /// limits, posting and a stock simulation of each setup. Writes the
    /// report to `path` if given.
    pub(crate) fn verify(
        &self,
        post: &str,
        machine: Option<&str>,
        setups: Option<&[u32]>,
        cell: Option<f64>,
        path: Option<&Path>,
    ) -> Result<Verification, ApiError> {
        let document = self.session.document();
        let cam = document.cam();
        let ids: Vec<u32> = match setups {
            Some(ids) => ids.to_vec(),
            None => cam.setups.iter().map(|s| s.id).collect(),
        };
        if ids.is_empty() {
            return Err(ApiError::new("cam", "there are no setups to verify"));
        }
        let mut problems = Vec::new();
        let mut review = Vec::new();
        let mut sections = String::new();
        let mut minutes = 0.0;

        // The program as it will be posted: plan, machine and post.
        let posted = if ids.len() == 1 {
            self.post_program(post, machine, ids.first().copied(), None, None, &[])
        } else {
            self.post_program(post, machine, None, Some(&ids), None, &[])
        };
        let lines = match &posted {
            Ok(value) => {
                for note in value["notes"].as_array().into_iter().flatten() {
                    if let Some(note) = note.as_str() {
                        review.push(format!("Note: {note}"));
                    }
                }
                value["gcode"].as_str().map_or(0, |g| g.lines().count())
            }
            Err(error) => {
                problems.push(format!("No G-code: {}", error.message));
                0
            }
        };

        // Each setup in turn: its tools, operations and simulation.
        for id in &ids {
            let Some(entry) = cam.setups.iter().find(|s| s.id == *id) else {
                problems.push(format!("There is no setup {id}."));
                continue;
            };
            let name = &entry.setup.name;
            let _ = writeln!(
                sections,
                "\n{name} · G{}{}",
                53 + u32::from(entry.setup.work_offset),
                entry
                    .setup
                    .index
                    .map_or(String::new(), |b| format!(" · table at B{b}"))
            );
            match cndt_process_mill::plan_setup(
                document,
                &self.rebuilt,
                self.kernel.as_ref(),
                Some(*id),
            ) {
                Ok(planned) => {
                    let mut tools: Vec<String> = Vec::new();
                    let mut typical = false;
                    for step in planned.program.operations.iter().flat_map(|o| &o.steps) {
                        if let Step::Event(Event::ToolChange {
                            number,
                            description,
                        }) = step
                        {
                            let mut tool = format!("T{number} {description}");
                            if let Some(t) = cam.tools.get(number) {
                                let a = t.assembly();
                                let _ = write!(
                                    tool,
                                    " (flutes {:.0}, sticks out {:.0}, holder Ø{:.0}{})",
                                    a.flute_length,
                                    a.stickout,
                                    a.holder_diameter,
                                    if t.assembly.is_none() {
                                        ", typical"
                                    } else {
                                        ""
                                    }
                                );
                                typical |= t.assembly.is_none();
                            }
                            if !tools.contains(&tool) {
                                tools.push(tool);
                            }
                        }
                    }
                    let _ = writeln!(sections, "  Tools:");
                    for tool in &tools {
                        let _ = writeln!(sections, "    {tool}");
                    }
                    if typical {
                        let note = String::from(
                            "Some tools have typical, not measured, flute lengths, stick-outs and holders: the shank and holder checks are only as good as those. Measure the tools and enter them.",
                        );
                        if !review.contains(&note) {
                            review.push(note);
                        }
                    }
                    let operations: Vec<&str> = planned
                        .program
                        .operations
                        .iter()
                        .map(|o| o.name.as_str())
                        .collect();
                    let _ = writeln!(sections, "  Operations: {}", operations.join(", "));
                }
                Err(error) => problems.push(format!("{name}: {error}")),
            }
            match self.simulate_setup(Some(*id), cell.or(Some(0.5))) {
                Ok(sim) => {
                    let report = &sim.report;
                    minutes += report.cut_minutes;
                    for hit in &report.collisions {
                        let what = match hit.what {
                            cndt_sim::Hit::Shank => {
                                "the shank above the flutes rubs the stock (the cut is deeper than the flutes are long)"
                            }
                            cndt_sim::Hit::Holder => {
                                "the holder hits the stock (the tool does not stick out far enough)"
                            }
                            cndt_sim::Hit::Rapid | cndt_sim::Hit::Gouge => {
                                "a rapid move runs into the stock"
                            }
                        };
                        problems.push(format!(
                            "{name}: {what}, {} deep, in {}, step {}, at {}.",
                            mm(hit.depth),
                            hit.operation,
                            hit.step,
                            at(hit.at)
                        ));
                    }
                    for gouge in &report.gouges {
                        review.push(format!(
                            "{name}: {} cuts {} below the model at {}.",
                            gouge.operation,
                            mm(gouge.depth),
                            at(gouge.at)
                        ));
                    }
                    if report.left > 0.05 {
                        let place = report
                            .left_at
                            .map_or(String::new(), |p| format!(" at {}", at(p)));
                        review.push(format!(
                            "{name}: up to {} of material is left on the part{place}.",
                            mm(report.left)
                        ));
                    }
                    let _ = writeln!(
                        sections,
                        "  Simulated: {} rapid{} into the stock, {} operation{} below the model, up to {} left, cutting {:.1} min",
                        report.collisions.len(),
                        if report.collisions.len() == 1 {
                            ""
                        } else {
                            "s"
                        },
                        report.gouges.len(),
                        if report.gouges.len() == 1 { "" } else { "s" },
                        mm(report.left),
                        report.cut_minutes
                    );
                }
                // Not a block: nothing seen is wrong, but nothing was checked.
                Err(error) => review.push(format!("{name}: not simulated: {}", error.message)),
            }
        }

        let verdict = if !problems.is_empty() {
            Verdict::Problems
        } else if !review.is_empty() {
            Verdict::Review
        } else {
            Verdict::Clear
        };
        let mut text = String::new();
        let _ = writeln!(text, "CNDT VERIFICATION REPORT");
        let _ = writeln!(text, "Part: {}", document.name());
        let _ = writeln!(
            text,
            "Post: {post}{}",
            machine.map_or(String::new(), |m| format!("    Machine: {m}"))
        );
        let _ = writeln!(
            text,
            "Program: {lines} lines, about {minutes:.1} min of cutting (rapids and tool changes not counted)"
        );
        let _ = writeln!(text, "CNDT {}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(
            text,
            "Everything here is UNVERIFIED until the program has cut a part. Prove it out carefully."
        );
        let _ = writeln!(
            text,
            "\nVERDICT: {}",
            match verdict {
                Verdict::Problems => format!(
                    "{} PROBLEM{} - DO NOT RUN until resolved",
                    problems.len(),
                    if problems.len() == 1 { "" } else { "S" }
                ),
                Verdict::Review => format!(
                    "NEEDS REVIEW - {} finding{} to check",
                    review.len(),
                    if review.len() == 1 { "" } else { "s" }
                ),
                Verdict::Clear => String::from("NO PROBLEMS FOUND"),
            }
        );
        for (title, items) in [("PROBLEMS", &problems), ("REVIEW", &review)] {
            if !items.is_empty() {
                let _ = writeln!(text, "\n{title}");
                for item in items {
                    let _ = writeln!(text, "  - {item}");
                }
            }
        }
        let _ = write!(text, "\nSETUPS{sections}");
        if let Some(path) = path {
            std::fs::write(path, &text).map_err(|e| {
                ApiError::new("io", format!("cannot write {}: {e}", path.display()))
            })?;
        }
        Ok(Verification {
            verdict,
            problems,
            review,
            text,
        })
    }
}
