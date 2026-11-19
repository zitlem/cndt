// SPDX-License-Identifier: AGPL-3.0-or-later
//! The setup sheet: one printable page per program for the operator to keep
//! at the machine for the first cut. For each setup it lists the work
//! offset, where zero is, the stock, how the side is reached, the
//! operations, the tools with their cutting data, the simulated cutting
//! time and every verification finding.
//!
//! A single self-contained HTML file with inline CSS that prints on A4 and
//! Letter. The same project gives the same bytes: no date, no time, no
//! machine-specific text. Like the programs, it says UNVERIFIED.

use std::fmt::Write as _;
use std::path::Path;

use cndt_cam_core::{Event, Step};
use cndt_model::cam::{Side, Tool, ToolKind};

use crate::verify::Verdict;
use crate::{Api, ApiError, PostRef};

/// The page, and how many setups it covers.
pub(crate) struct Sheet {
    pub(crate) html: String,
    pub(crate) setups: usize,
}

/// Text made safe to put inside HTML.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// A length in millimetres, without trailing noise: `10`, `12.5`.
fn num(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn kind_name(kind: &ToolKind) -> String {
    match kind {
        ToolKind::SpotDrill => String::from("spot drill"),
        ToolKind::Drill => String::from("drill"),
        ToolKind::Tap { thread } => format!("tap {thread}"),
        ToolKind::Reamer => String::from("reamer"),
        ToolKind::EndMill => String::from("end mill"),
        ToolKind::FaceMill => String::from("face mill"),
        ToolKind::ChamferMill => String::from("chamfer mill"),
    }
}

/// How the side is reached, in words an operator can follow.
fn reach(side: Side, index: Option<f64>, flip_y: bool, angle: Option<f64>) -> String {
    let which = match side {
        Side::Top => String::from("top face to the spindle, as modelled"),
        Side::Bottom => format!(
            "bottom face to the spindle: part turned over about {}",
            if flip_y { "Y" } else { "X" }
        ),
        Side::Front => String::from("front face (model -Y) to the spindle"),
        Side::Back => String::from("back face (model +Y) to the spindle"),
        Side::Left => String::from("left face (model -X) to the spindle"),
        Side::Right => String::from("right face (model +X) to the spindle"),
        Side::Turned => format!(
            "the face {}° round from the top about model Y to the spindle",
            num(angle.unwrap_or(0.0))
        ),
    };
    match index {
        Some(angle) => format!("{which}; the B table turns to B{}", num(angle)),
        None => format!("{which}; turned by hand"),
    }
}

/// One tool as the sheet lists it.
struct ToolRow {
    number: u32,
    description: String,
    kind: String,
    tool: Option<Tool>,
}

impl Api {
    /// Builds the setup sheet for the given setups (all of them, in order,
    /// if `None`), posted through `post` on `machine`. Runs the same
    /// verification the post does, so every finding is on the page.
    pub(crate) fn setup_sheet(
        &self,
        post: &PostRef,
        machine: Option<&str>,
        setups: Option<&[u32]>,
        path: Option<&Path>,
    ) -> Result<Sheet, ApiError> {
        // A post that does not exist is a bad request, not a sheet full of problems.
        post.load()?;
        let document = self.session.document();
        let cam = document.cam();
        let ids: Vec<u32> = match setups {
            Some(ids) => ids.to_vec(),
            None => cam.setups.iter().map(|s| s.id).collect(),
        };
        if ids.is_empty() {
            return Err(ApiError::new(
                "cam",
                "there are no setups to put on a sheet",
            ));
        }
        let cam_error = |e: cndt_process_mill::CamError| ApiError::new("cam", e.to_string());
        let program =
            cndt_process_mill::plan_setups(document, &self.rebuilt, self.kernel.as_ref(), &ids)
                .map_err(cam_error)?
                .program
                .name;
        let machine_def = machine.map(crate::shipped_machine).transpose()?;
        let report = self.verify(post, machine, Some(&ids), None, None)?;
        let sims = self.simulate_chain(&ids, None)?;

        let mut body = String::new();
        let mut total = 0.0;
        for (id, sim) in ids.iter().zip(sims) {
            let Some(entry) = cam.setups.iter().find(|s| s.id == *id) else {
                return Err(ApiError::new("cam", format!("there is no setup {id}")));
            };
            let def = &entry.setup;
            let planned = cndt_process_mill::plan_setup(
                document,
                &self.rebuilt,
                self.kernel.as_ref(),
                Some(*id),
            )
            .map_err(cam_error)?;
            let _ = write!(
                body,
                "<section class=\"setup\">\n<h2>{} <span>G{} work offset</span></h2>\n<table class=\"facts\">\n",
                esc(&def.name),
                53 + u32::from(def.work_offset)
            );
            let fact = |body: &mut String, label: &str, value: &str| {
                let _ = writeln!(
                    body,
                    "<tr><th>{}</th><td>{}</td></tr>",
                    esc(label),
                    esc(value)
                );
            };
            fact(
                &mut body,
                "Side and turning",
                &reach(
                    def.side,
                    def.index,
                    def.flip_about == cndt_model::cam::FlipAxis::Y,
                    def.angle,
                ),
            );
            fact(
                &mut body,
                "Vise",
                &def.vise.map_or_else(
                    || String::from("none entered: no fixture was checked"),
                    |v| {
                        format!(
                            "jaws on the {}, gripping {} mm of the stock from its bottom up, {} mm thick, \
                             the stock on {} mm parallels under its clamped faces; every tool was \
                             checked against them",
                            match v.along {
                                cndt_model::cam::JawAxis::X => "front and back",
                                cndt_model::cam::JawAxis::Y => "left and right",
                            },
                            num(v.grip),
                            num(v.jaw),
                            num(cndt_model::cam::PARALLEL)
                        )
                    },
                ),
            );
            fact(
                &mut body,
                "Where zero is",
                "X0 Y0 at the corner of the finished part nearest the left front, as the spindle \
                 sees it. Z0 on the finished face the spindle sees.",
            );
            if let Some(placed) = &planned.setup {
                // In the model's own coordinates: for a side setup the
                // origin is on the turned part (decision 0019).
                let zero = placed.in_model(cndt_geom::Point3::new(0.0, 0.0, 0.0));
                fact(
                    &mut body,
                    "Zero in model coordinates",
                    &format!("X{} Y{} Z{}", num(zero.x), num(zero.y), num(zero.z)),
                );
                let stock = cndt_process_mill::world(
                    document,
                    &self.rebuilt,
                    self.kernel.as_ref(),
                    placed.turn,
                )
                .and_then(|w| cndt_process_mill::stock_box(&w.document, &w.rebuilt, placed));
                match (&cam.stock, stock) {
                    (Some(a), Ok(b)) => fact(
                        &mut body,
                        "Stock",
                        &format!(
                            "{} x {} x {} mm; allowances: sides {}, top {}, bottom {} mm",
                            num(b.max.x - b.min.x),
                            num(b.max.y - b.min.y),
                            num(b.max.z - b.min.z),
                            num(a.side),
                            num(a.top),
                            num(a.bottom)
                        ),
                    ),
                    (None, _) => fact(&mut body, "Stock", "not set"),
                    (_, Err(error)) => fact(&mut body, "Stock", &error.to_string()),
                }
            }
            let _ = writeln!(body, "</table>");

            // Operations in order, and the tools they use, in order of first use.
            let mut tools: Vec<ToolRow> = Vec::new();
            for step in planned.program.operations.iter().flat_map(|o| &o.steps) {
                let Step::Event(Event::ToolChange {
                    number,
                    description,
                }) = step
                else {
                    continue;
                };
                if tools.iter().all(|t| t.number != *number) {
                    let tool = cam.tools.get(number).cloned();
                    tools.push(ToolRow {
                        number: *number,
                        description: description.clone(),
                        kind: tool
                            .as_ref()
                            .map_or_else(String::new, |t| kind_name(&t.kind)),
                        tool,
                    });
                }
            }
            let _ = writeln!(body, "<h3>Operations, in order</h3>\n<ol>");
            for op in &planned.program.operations {
                let _ = writeln!(body, "<li>{}</li>", esc(&op.name));
            }
            let _ = writeln!(body, "</ol>\n<h3>Tools</h3>");
            let _ = writeln!(
                body,
                "<table class=\"grid\">\n<tr><th>T</th><th>Tool</th><th>Kind</th><th>Dia mm</th><th>rpm</th>\
                 <th>Feed mm/min</th><th>Sticks out mm</th><th>Holder dia mm</th><th>Assembly</th></tr>"
            );
            for row in &tools {
                match &row.tool {
                    Some(t) => {
                        let a = t.assembly();
                        let _ = writeln!(
                            body,
                            "<tr><td>T{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                            row.number,
                            esc(&row.description),
                            esc(&row.kind),
                            num(t.diameter),
                            num(t.rpm),
                            num(t.feed),
                            num(a.stickout),
                            num(a.holder_diameter),
                            if t.assembly.is_none() {
                                "<b class=\"typical\">TYPICAL</b> (not measured)"
                            } else {
                                "measured"
                            }
                        );
                    }
                    None => {
                        let _ = writeln!(
                            body,
                            "<tr><td>T{}</td><td>{}</td><td colspan=\"7\">not in the tool library</td></tr>",
                            row.number,
                            esc(&row.description)
                        );
                    }
                }
            }
            let _ = writeln!(body, "</table>");
            match sim {
                Ok(sim) => {
                    total += sim.report.cut_minutes;
                    let _ = writeln!(
                        body,
                        "<p class=\"time\">Estimated cutting time: <b>{:.1} min</b> from the stock simulation (rapids and tool changes not counted).</p>",
                        sim.report.cut_minutes
                    );
                }
                Err(error) => {
                    let _ = writeln!(
                        body,
                        "<p class=\"time\">Not simulated: {}</p>",
                        esc(&error.message)
                    );
                }
            }
            let _ = writeln!(body, "</section>");
        }

        let (verdict, class) = match report.verdict {
            Verdict::Problems => ("PROBLEMS: DO NOT RUN THE PROGRAM", "bad"),
            Verdict::Review => ("NEEDS REVIEW", "warn"),
            Verdict::Clear => (
                "NO PROBLEMS FOUND (still unverified until it has cut)",
                "ok",
            ),
        };
        let mut findings = String::new();
        for (title, list) in [
            ("Problems", &report.problems),
            ("To review", &report.review),
        ] {
            if list.is_empty() {
                continue;
            }
            let _ = writeln!(findings, "<h3>{title}</h3>\n<ul>");
            for item in list {
                let _ = writeln!(findings, "<li>{}</li>", esc(item));
            }
            let _ = writeln!(findings, "</ul>");
        }
        if findings.is_empty() {
            findings.push_str("<p>No findings.</p>\n");
        }

        let (machine_name, machine_note) = match &machine_def {
            Some(m) => (
                m.machine.name.clone(),
                if m.machine.verified {
                    ""
                } else {
                    " (machine definition not verified)"
                },
            ),
            None => (
                String::from("none chosen: machine limits were not checked"),
                "",
            ),
        };
        let mut html = String::new();
        let _ = write!(
            html,
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <title>Setup sheet: {part} (UNVERIFIED)</title>\n<style>\n{CSS}</style>\n</head>\n<body>\n\
             <h1>SETUP SHEET <em>UNVERIFIED</em></h1>\n\
             <p class=\"banner\">Made by CNDT {version}. Nothing here has been proved on a machine. \
             Prove the program out carefully: air cut first, hand on the feed hold.</p>\n\
             <table class=\"facts\">\n\
             <tr><th>Part</th><td>{part}</td></tr>\n\
             <tr><th>Program</th><td>{program}</td></tr>\n\
             <tr><th>Post</th><td>{post}</td></tr>\n\
             <tr><th>Machine</th><td>{machine}{machine_note}</td></tr>\n\
             <tr><th>Setups</th><td>{count}, in this order</td></tr>\n\
             <tr><th>Cutting time</th><td>about {total:.1} min in all</td></tr>\n\
             <tr><th>Verification</th><td class=\"{class}\">{verdict}</td></tr>\n\
             </table>\n{findings}{body}\
             <section class=\"sign\">\n<h3>At the machine</h3>\n\
             <table class=\"grid\">\n\
             <tr><th>Work offsets touched off and checked</th><td></td></tr>\n\
             <tr><th>Tool lengths and stick-outs measured</th><td></td></tr>\n\
             <tr><th>Air cut done</th><td></td></tr>\n\
             <tr><th>Checked by, date</th><td></td></tr>\n\
             </table>\n</section>\n</body>\n</html>\n",
            part = esc(document.name()),
            version = env!("CARGO_PKG_VERSION"),
            program = esc(&program),
            post = esc(&post.to_string()),
            machine = esc(&machine_name),
            count = ids.len(),
        );
        if let Some(path) = path {
            std::fs::write(path, &html).map_err(|e| {
                ApiError::new("io", format!("cannot write {}: {e}", path.display()))
            })?;
        }
        Ok(Sheet {
            html,
            setups: ids.len(),
        })
    }
}

/// Plain, printable, light on ink. No `@page size`, so the printer's own
/// paper (A4 or Letter) decides; margins in millimetres suit both.
const CSS: &str = "\
@page { margin: 12mm; }
* { box-sizing: border-box; }
body { font: 10pt/1.35 system-ui, -apple-system, \"Segoe UI\", Helvetica, Arial, sans-serif; color: #000; background: #fff; margin: 0 auto; max-width: 190mm; padding: 6mm; }
h1 { font-size: 18pt; margin: 0 0 4pt; }
h1 em { font-style: normal; border: 2pt solid #000; padding: 0 5pt; margin-left: 8pt; font-size: 12pt; vertical-align: 3pt; }
h2 { font-size: 13pt; margin: 14pt 0 4pt; border-bottom: 1.5pt solid #000; }
h2 span { font-weight: normal; font-size: 10pt; margin-left: 6pt; }
h3 { font-size: 10.5pt; margin: 10pt 0 3pt; }
p { margin: 3pt 0; }
.banner { border: 1pt solid #000; padding: 4pt 6pt; }
table { border-collapse: collapse; width: 100%; margin: 3pt 0; }
th, td { border: 0.6pt solid #555; padding: 2.5pt 5pt; text-align: left; vertical-align: top; }
.facts th { width: 38mm; background: #eee; }
.grid th { background: #eee; font-size: 9pt; }
.sign th { width: 70mm; }
.sign td { height: 9mm; }
.bad { font-weight: bold; text-decoration: underline; }
.warn { font-weight: bold; }
.typical { border: 1pt solid #000; padding: 0 3pt; }
ul, ol { margin: 2pt 0; padding-left: 16pt; }
.setup { break-inside: avoid; }
.time { margin-top: 4pt; }
@media print { body { padding: 0; max-width: none; } }
";
