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

/// How far the stock the posted G-code leaves may differ from what the
/// toolpath leaves before it is a problem, in millimetres: the posts round
/// to 0.001 mm.
const READ_BACK_TOLERANCE: f64 = 0.01;

impl Api {
    /// Posts one setup alone, reads the G-code back and simulates it on the
    /// stock `sim` started from. `Ok(None)` when it cuts what the toolpath
    /// cuts; `Ok(Some(problem))` when it does not; `Err` when it could not
    /// be checked (a post that writes no tool numbers, say).
    fn read_back_check(
        &self,
        post: &crate::PostRef,
        machine: Option<&str>,
        id: u32,
        sim: &crate::simulate::Simulated,
        planned: Option<&cndt_cam_core::Program>,
    ) -> Result<Option<String>, String> {
        let posted = self
            .post_program(post, machine, Some(id), None, crate::Output::default(), &[])
            .map_err(|e| e.message)?;
        let gcode = posted["gcode"].as_str().unwrap_or_default();
        let back = cndt_post::read_back(gcode).map_err(|e| e.to_string())?;
        let report = cndt_sim::simulate_controlled(&back, &sim.tools, &sim.input, &self.control)
            .map_err(|e| format!("it could not be simulated: {e}"))?;
        // Material removed is not all: a wrong feed, or a cut posted as a
        // rapid, removes the same stock.
        if let Some(planned) = planned
            && let Some(problem) = crate::motion_check::compare(planned, &back)
        {
            return Ok(Some(problem));
        }
        let worst = report
            .heights
            .iter()
            .zip(&sim.report.heights)
            .enumerate()
            .map(|(c, (a, b))| (c, (a - b).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        Ok(match worst {
            Some((c, d)) if d > READ_BACK_TOLERANCE => {
                let [x, y] = sim.input.grid.centres()[c];
                Some(format!(
                    "the posted G-code does not cut what was planned: the stock it leaves \
                     differs by {} at program X{x:.2} Y{y:.2}. The post may be wrong; do not run it.",
                    mm(d)
                ))
            }
            _ if report.heights.len() != sim.report.heights.len() => Some(String::from(
                "the posted G-code could not be compared with the plan",
            )),
            _ => None,
        })
    }

    /// Verifies the given setups (all of them, in order, if `None`) as one
    /// program through `post`, on `machine` if given: plans, machine
    /// limits, posting and a stock simulation of each setup. Writes the
    /// report to `path` if given.
    pub(crate) fn verify(
        &self,
        post: &crate::PostRef,
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
            self.post_program(
                post,
                machine,
                ids.first().copied(),
                None,
                crate::Output::default(),
                &[],
            )
        } else {
            self.post_program(
                post,
                machine,
                None,
                Some(&ids),
                crate::Output::default(),
                &[],
            )
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

        // Each setup in turn: its tools, operations and simulation, each
        // simulated on the stock the ones before it left.
        let mut sims = self.simulate_chain(&ids, cell.or(Some(0.5)))?.into_iter();
        for id in &ids {
            // A cancelled verification is not a failed one: no report.
            if self.control.is_cancelled() {
                return Err(crate::simulate::sim_error(cndt_sim::SimError::Cancelled));
            }
            let simulated = sims
                .next()
                .unwrap_or_else(|| Err(ApiError::new("internal", "not simulated")));
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
            let mut planned_program = None;
            match cndt_process_mill::plan_setup(
                document,
                &self.rebuilt,
                self.kernel.as_ref(),
                Some(*id),
            ) {
                Ok(planned) => {
                    planned_program = Some(planned.program.clone());
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
            match simulated {
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
                            cndt_sim::Hit::Fixture => {
                                "the tool hits the vise (a jaw, a parallel under the stock's clamped edge, or its floor)"
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
                    // The G-code as posted, read back and simulated on the
                    // same stock: it must cut exactly what was planned.
                    match self.read_back_check(post, machine, *id, &sim, planned_program.as_ref()) {
                        Ok(None) => {
                            let _ = writeln!(
                                sections,
                                "  Posted G-code read back and simulated: it cuts what was planned"
                            );
                        }
                        Ok(Some(problem)) => problems.push(format!("{name}: {problem}")),
                        Err(reason) => review.push(format!(
                            "{name}: the posted G-code was not read back to check it: {reason}"
                        )),
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
