// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 3 post-processor engine: turns a neutral [`Program`] into G-code for
//! one controller.
//!
//! A post is a plain-text TOML file (see `posts/` at the repository root)
//! that a shop can read, edit, version and share. This is the declarative
//! layer; a sandboxed script hook for unusual controllers comes later.
//!
//! Safety (docs/safety.md):
//! - Programs are validated before posting; an unsafe program is refused.
//! - Every program carries a header naming the app version, the post, its
//!   version and whether it has been verified on a real machine, plus a
//!   not-for-production notice for preview builds. A post cannot leave the
//!   header out: its `start` lines must contain `{header}`.
//! - Templates are checked when the post loads, so a typo in a post fails
//!   at once instead of producing wrong G-code.
//! - Output is deterministic: the same program and post give the same bytes.

use std::fmt;

use cndt_cam_core::{
    ArcDirection, Coolant, Event, Move, Program, SpindleDirection, Step, ToolpathError,
};
use cndt_geom::Point3;
use serde::Deserialize;

/// A post definition, as read from TOML.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostDefinition {
    /// Identity and verification status.
    pub post: PostInfo,
    /// Number and line formatting.
    pub format: Format,
    /// Lines at the start and end of every program.
    pub program: ProgramLines,
    /// The codes for each kind of step.
    pub codes: Codes,
}

/// Identity and verification status.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostInfo {
    /// A short stable identifier, such as `fanuc-mill`.
    pub id: String,
    /// A readable name.
    pub name: String,
    /// The post's own version.
    pub version: String,
    /// `true` only after its output has run on a real controller.
    pub verified: bool,
}

/// Number and line formatting.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    /// Decimal places for coordinates, in millimetres.
    pub decimals: usize,
    /// Decimal places for feed rates.
    pub feed_decimals: usize,
    /// Drop trailing zeros after the decimal point.
    pub trim_zeros: bool,
    /// Keep the decimal point on whole numbers (`10.` rather than `10`).
    /// Many controllers read `X10` as 10 thousandths.
    pub keep_decimal_point: bool,
    /// Block numbering, if the controller wants it.
    pub line_numbers: Option<LineNumbers>,
    /// How comments are written.
    pub comment: CommentStyle,
    /// Wrap the program in `%` lines.
    pub percent_wrapped: bool,
    /// Zero-padded width of `{program_number}`.
    pub program_number_digits: usize,
}

/// Block numbering.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineNumbers {
    /// The first block number.
    pub start: u32,
    /// The increment.
    pub step: u32,
}

/// How comments are written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentStyle {
    /// Text before the comment, such as `(`.
    pub open: String,
    /// Text after the comment, such as `)`.
    pub close: String,
    /// Convert comment text to upper case.
    pub uppercase: bool,
}

/// Lines at the start and end of every program.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramLines {
    /// An optional first line, written without a block number, such as
    /// Fanuc's `O1001 (NAME)`. Same placeholders as `start`.
    #[serde(default)]
    pub first_line: Option<String>,
    /// Lines at the start. One must be exactly `{header}`. May use
    /// `{program_number}`, `{program_name}` and `{work_offset}`.
    pub start: Vec<String>,
    /// Lines at the end.
    pub end: Vec<String>,
}

/// The codes for each kind of step.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Codes {
    /// Rapid motion, such as `G0`.
    pub rapid: String,
    /// Linear cutting, such as `G1`.
    pub linear: String,
    /// Clockwise arc, such as `G2`.
    pub arc_cw: String,
    /// Counter-clockwise arc, such as `G3`.
    pub arc_ccw: String,
    /// Spindle on clockwise. Uses `{rpm}`.
    pub spindle_cw: String,
    /// Spindle on counter-clockwise. Uses `{rpm}`.
    pub spindle_ccw: String,
    /// Spindle off.
    pub spindle_off: String,
    /// Flood coolant on.
    pub coolant_flood: String,
    /// Mist coolant on.
    pub coolant_mist: String,
    /// Coolant off.
    pub coolant_off: String,
    /// Program stop.
    pub stop: String,
    /// Optional stop.
    pub optional_stop: String,
    /// Dwell. Uses `{seconds}` or `{milliseconds}`.
    pub dwell: String,
    /// Lines for a tool change. Use `{tool}`.
    pub tool_change: Vec<String>,
    /// Tool length compensation, such as `G43 H{tool}`. When set, the first
    /// move after every tool change must be a rapid; it is split into an XY
    /// move, then this code with the Z move, so Z only moves with the
    /// tool's length applied. Leave out for controllers without it.
    #[serde(default)]
    pub tool_length_compensation: Option<String>,
}

/// Why posting failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostError {
    /// The post definition is malformed.
    InvalidDefinition(String),
    /// The program is not safe to post.
    Toolpath(ToolpathError),
    /// The program is valid, but this post cannot output it safely.
    Unsupported(String),
}

impl fmt::Display for PostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition(reason) => write!(f, "invalid post definition: {reason}"),
            Self::Toolpath(error) => write!(f, "the program cannot be posted: {error}"),
            Self::Unsupported(reason) => write!(f, "this post cannot output the program: {reason}"),
        }
    }
}

impl std::error::Error for PostError {}

/// A loaded, checked post.
#[derive(Debug, Clone)]
pub struct Post {
    definition: PostDefinition,
}

impl Post {
    /// Loads a post from TOML and checks every template in it.
    ///
    /// # Errors
    ///
    /// Returns [`PostError::InvalidDefinition`] if the TOML is malformed, has
    /// unknown or missing fields, a template uses an unknown placeholder, or
    /// the start lines leave out `{header}`.
    pub fn from_toml(text: &str) -> Result<Self, PostError> {
        let definition: PostDefinition = toml::from_str(text)
            .map_err(|error| PostError::InvalidDefinition(error.to_string()))?;
        check_definition(&definition)?;
        Ok(Self { definition })
    }

    /// The definition this post was loaded from.
    #[must_use]
    pub const fn definition(&self) -> &PostDefinition {
        &self.definition
    }

    /// Posts a program.
    ///
    /// # Errors
    ///
    /// Returns [`PostError::Toolpath`] if the program fails validation. No
    /// G-code is produced for an unsafe program.
    pub fn run(&self, program: &Program, app_version: &str) -> Result<String, PostError> {
        program.validate().map_err(PostError::Toolpath)?;
        let mut out = Output::new(&self.definition);
        let def = &self.definition;

        if def.format.percent_wrapped {
            out.raw("%");
        }
        let program_number = format!(
            "{:0width$}",
            program.number,
            width = def.format.program_number_digits
        );
        let program_name = out.comment_text(&program.name);
        let work_offset = format!("G{}", 53 + u32::from(program.work_offset));
        let values = [
            ("program_number", program_number.as_str()),
            ("program_name", program_name.as_str()),
            ("work_offset", work_offset.as_str()),
        ];
        if let Some(line) = &def.program.first_line {
            out.raw(&fill(line, &values));
        }
        for line in &def.program.start {
            if line == "{header}" {
                for text in header(def, app_version) {
                    out.comment(&text);
                }
            } else {
                out.block(&fill(line, &values));
            }
        }

        for operation in &program.operations {
            out.comment(&operation.name);
            for (index, step) in operation.steps.iter().enumerate() {
                match step {
                    Step::Move(motion) => out.motion(motion).map_err(|reason| {
                        PostError::Unsupported(format!(
                            "{}, step {}: {reason}",
                            operation.name,
                            index + 1
                        ))
                    })?,
                    Step::Event(event) => out.event(event),
                }
            }
        }

        for line in &def.program.end {
            out.block(line);
        }
        if def.format.percent_wrapped {
            out.raw("%");
        }
        Ok(out.finish())
    }
}

/// The safety header (docs/safety.md, gates 5 and 6).
fn header(def: &PostDefinition, app_version: &str) -> Vec<String> {
    let status = if def.post.verified {
        "VERIFIED"
    } else {
        "UNVERIFIED"
    };
    let mut lines = vec![
        format!("CNDT {app_version}"),
        format!("POST {} {} {status}", def.post.id, def.post.version),
    ];
    if !def.post.verified {
        lines.push(String::from(
            "UNVERIFIED POST - PROVE OUT CAREFULLY BEFORE RUNNING",
        ));
    }
    if app_version.starts_with("0.") {
        lines.push(String::from("PREVIEW BUILD - NOT FOR PRODUCTION"));
    }
    lines
}

/// Placeholders each template may use.
fn check_definition(def: &PostDefinition) -> Result<(), PostError> {
    let invalid = |reason: String| PostError::InvalidDefinition(reason);
    if !def.program.start.iter().any(|line| line == "{header}") {
        return Err(invalid(String::from(
            "program.start must contain a line \"{header}\" for the safety header",
        )));
    }
    if def.format.decimals > 6 || def.format.feed_decimals > 6 {
        return Err(invalid(String::from("at most 6 decimals are supported")));
    }
    let start_keys = ["program_number", "program_name", "work_offset"];
    let checks: Vec<(&str, &str, &[&str])> = def
        .program
        .start
        .iter()
        .filter(|line| *line != "{header}")
        .map(|line| ("program.start", line.as_str(), &start_keys[..]))
        .chain(
            def.program
                .first_line
                .iter()
                .map(|line| ("program.first_line", line.as_str(), &start_keys[..])),
        )
        .chain(def.codes.tool_length_compensation.iter().map(|line| {
            (
                "codes.tool_length_compensation",
                line.as_str(),
                &["tool"][..],
            )
        }))
        .chain(
            def.program
                .end
                .iter()
                .map(|line| ("program.end", line.as_str(), &[][..])),
        )
        .chain(
            def.codes
                .tool_change
                .iter()
                .map(|line| ("codes.tool_change", line.as_str(), &["tool"][..])),
        )
        .chain([
            ("codes.rapid", def.codes.rapid.as_str(), &[][..]),
            ("codes.linear", def.codes.linear.as_str(), &[][..]),
            ("codes.arc_cw", def.codes.arc_cw.as_str(), &[][..]),
            ("codes.arc_ccw", def.codes.arc_ccw.as_str(), &[][..]),
            (
                "codes.spindle_cw",
                def.codes.spindle_cw.as_str(),
                &["rpm"][..],
            ),
            (
                "codes.spindle_ccw",
                def.codes.spindle_ccw.as_str(),
                &["rpm"][..],
            ),
            ("codes.spindle_off", def.codes.spindle_off.as_str(), &[][..]),
            (
                "codes.coolant_flood",
                def.codes.coolant_flood.as_str(),
                &[][..],
            ),
            (
                "codes.coolant_mist",
                def.codes.coolant_mist.as_str(),
                &[][..],
            ),
            ("codes.coolant_off", def.codes.coolant_off.as_str(), &[][..]),
            ("codes.stop", def.codes.stop.as_str(), &[][..]),
            (
                "codes.optional_stop",
                def.codes.optional_stop.as_str(),
                &[][..],
            ),
            (
                "codes.dwell",
                def.codes.dwell.as_str(),
                &["seconds", "milliseconds"][..],
            ),
        ])
        .collect();
    for (field, template, allowed) in checks {
        for key in placeholders(template).map_err(|reason| invalid(format!("{field}: {reason}")))? {
            if !allowed.contains(&key.as_str()) {
                return Err(invalid(format!(
                    "{field} uses {{{key}}}, which is not available there (allowed: {})",
                    if allowed.is_empty() {
                        String::from("none")
                    } else {
                        allowed.join(", ")
                    }
                )));
            }
        }
    }
    Ok(())
}

/// The placeholder names in a template, such as `rpm` in `M3 S{rpm}`.
fn placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after
            .find('}')
            .ok_or_else(|| format!("unclosed {{ in {template:?}"))?;
        keys.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
    if rest.contains('}') {
        return Err(format!("stray }} in {template:?}"));
    }
    Ok(keys)
}

/// Replaces each `{key}` in a template. Keys were checked at load time.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.to_owned();
    for (key, value) in values {
        text = text.replace(&format!("{{{key}}}"), value);
    }
    text
}

/// Formats a number deterministically, never as `-0`.
fn number(value: f64, decimals: usize, trim_zeros: bool, keep_point: bool) -> String {
    let mut text = format!("{value:.decimals$}");
    if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
        text.remove(0);
    }
    if trim_zeros && text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') && !keep_point {
            text.pop();
        }
    } else if keep_point && !text.contains('.') {
        text.push('.');
    }
    text
}

/// Builds the program text and tracks modal state.
struct Output<'a> {
    def: &'a PostDefinition,
    lines: Vec<String>,
    next_number: Option<u32>,
    position: Option<Point3>,
    motion: Option<String>,
    axes: [Option<String>; 3],
    feed: Option<String>,
    /// Set by a tool change when the post uses tool length compensation:
    /// the tool number still waiting for its compensation code.
    pending_length_compensation: Option<u32>,
}

impl<'a> Output<'a> {
    fn new(def: &'a PostDefinition) -> Self {
        Self {
            def,
            lines: Vec::new(),
            next_number: def.format.line_numbers.map(|numbers| numbers.start),
            position: None,
            motion: None,
            axes: [None, None, None],
            feed: None,
            pending_length_compensation: None,
        }
    }

    fn raw(&mut self, text: &str) {
        self.lines.push(text.to_owned());
    }

    fn block(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match (self.next_number, self.def.format.line_numbers) {
            (Some(current), Some(numbers)) => {
                self.lines.push(format!("N{current} {text}"));
                self.next_number = Some(current.saturating_add(numbers.step));
            }
            _ => self.lines.push(text.to_owned()),
        }
    }

    /// Text made safe to sit inside a comment.
    fn comment_text(&self, text: &str) -> String {
        let style = &self.def.format.comment;
        let cleaned: String = text
            .chars()
            .map(|c| {
                let delimiter = style.open.contains(c) || style.close.contains(c);
                if delimiter || !(c.is_ascii_graphic() || c == ' ') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if style.uppercase {
            cleaned.to_ascii_uppercase()
        } else {
            cleaned
        }
    }

    fn comment(&mut self, text: &str) {
        let style = &self.def.format.comment;
        let line = format!("{}{}{}", style.open, self.comment_text(text), style.close);
        self.lines.push(line);
    }

    fn coordinate(&self, value: f64) -> String {
        let format = &self.def.format;
        number(
            value,
            format.decimals,
            format.trim_zeros,
            format.keep_decimal_point,
        )
    }

    /// Forgets modal state, after anything that may change it behind our back.
    fn reset_modal(&mut self) {
        self.motion = None;
        self.axes = [None, None, None];
        self.feed = None;
    }

    fn motion(&mut self, motion: &Move) -> Result<(), String> {
        if let Some(tool) = self.pending_length_compensation {
            match motion {
                Move::Rapid { to } => {
                    self.compensated_approach(tool, *to);
                    return Ok(());
                }
                Move::Dwell { .. } => {}
                Move::Linear { .. } | Move::Arc { .. } => {
                    return Err(String::from(
                        "the first move after a tool change must be a rapid, so tool length \
                         compensation is applied before cutting",
                    ));
                }
            }
        }
        let codes = &self.def.codes;
        match motion {
            Move::Rapid { to } => self.linear_block(&codes.rapid.clone(), *to, None),
            Move::Linear { to, feed } => self.linear_block(&codes.linear.clone(), *to, Some(*feed)),
            Move::Arc {
                to,
                center,
                direction,
                feed,
            } => {
                let code = match direction {
                    ArcDirection::Clockwise => codes.arc_cw.clone(),
                    ArcDirection::CounterClockwise => codes.arc_ccw.clone(),
                };
                // Validation guarantees a known start before any arc.
                let start = self.position.unwrap_or(*to);
                let mut words = self.motion_words(&code, *to, true);
                words.push(format!("I{}", self.coordinate(center.x - start.x)));
                words.push(format!("J{}", self.coordinate(center.y - start.y)));
                words.extend(self.feed_word(*feed));
                self.block(&words.join(" "));
                self.position = Some(*to);
            }
            Move::Dwell { seconds } => {
                let line = fill(
                    &codes.dwell,
                    &[
                        ("seconds", &number(*seconds, 3, true, false)),
                        ("milliseconds", &number(seconds * 1000.0, 0, true, false)),
                    ],
                );
                self.block(&line);
            }
        }
        Ok(())
    }

    /// The first rapid after a tool change: X and Y first, then Z together
    /// with the tool length compensation code, so Z never moves without it.
    fn compensated_approach(&mut self, tool: u32, to: Point3) {
        let rapid = self.def.codes.rapid.clone();
        let mut words = vec![rapid.clone()];
        for (axis, (letter, value)) in [("X", to.x), ("Y", to.y)].into_iter().enumerate() {
            let text = self.coordinate(value);
            words.push(format!("{letter}{text}"));
            self.axes[axis] = Some(text);
        }
        self.block(&words.join(" "));
        self.motion = Some(rapid);

        let compensation = self
            .def
            .codes
            .tool_length_compensation
            .clone()
            .unwrap_or_default();
        let z = self.coordinate(to.z);
        self.block(&format!(
            "{} Z{z}",
            fill(&compensation, &[("tool", &tool.to_string())])
        ));
        self.axes[2] = Some(z);
        self.position = Some(to);
        self.pending_length_compensation = None;
    }

    fn linear_block(&mut self, code: &str, to: Point3, feed: Option<f64>) {
        if self.axes_unchanged(to) {
            // A move to where the tool already is. Output nothing, and leave
            // the modal state alone, so the next move still prints any code
            // that changed.
            self.position = Some(to);
            return;
        }
        let mut words = self.motion_words(code, to, false);
        if let Some(feed) = feed {
            words.extend(self.feed_word(feed));
        }
        self.block(&words.join(" "));
        self.position = Some(to);
    }

    fn axes_unchanged(&self, to: Point3) -> bool {
        let values = [to.x, to.y, to.z];
        (0..3)
            .all(|axis| self.axes[axis].as_deref() == Some(self.coordinate(values[axis]).as_str()))
    }

    /// The motion code (if it changed) and every axis word that changed.
    /// Arcs always repeat their code and their X and Y.
    fn motion_words(&mut self, code: &str, to: Point3, arc: bool) -> Vec<String> {
        let mut words = Vec::new();
        if arc || self.motion.as_deref() != Some(code) {
            words.push(code.to_owned());
            self.motion = Some(code.to_owned());
        }
        let values = [to.x, to.y, to.z];
        for (axis, letter) in ["X", "Y", "Z"].iter().enumerate() {
            let text = self.coordinate(values[axis]);
            let force = arc && axis < 2;
            if force || self.axes[axis].as_deref() != Some(text.as_str()) {
                words.push(format!("{letter}{text}"));
                self.axes[axis] = Some(text);
            }
        }
        words
    }

    fn feed_word(&mut self, feed: f64) -> Option<String> {
        let format = &self.def.format;
        let text = number(
            feed,
            format.feed_decimals,
            format.trim_zeros,
            format.keep_decimal_point,
        );
        if self.feed.as_deref() == Some(text.as_str()) {
            None
        } else {
            self.feed = Some(text.clone());
            Some(format!("F{text}"))
        }
    }

    fn event(&mut self, event: &Event) {
        let codes = self.def.codes.clone();
        match event {
            Event::ToolChange {
                number: tool,
                description,
            } => {
                self.comment(&format!("T{tool} {description}"));
                for line in &codes.tool_change {
                    self.block(&fill(line, &[("tool", &tool.to_string())]));
                }
                self.reset_modal();
                if codes.tool_length_compensation.is_some() {
                    self.pending_length_compensation = Some(*tool);
                }
            }
            Event::SpindleOn { rpm, direction } => {
                let template = match direction {
                    SpindleDirection::Clockwise => &codes.spindle_cw,
                    SpindleDirection::CounterClockwise => &codes.spindle_ccw,
                };
                self.block(&fill(template, &[("rpm", &number(*rpm, 0, true, false))]));
            }
            Event::SpindleOff => self.block(&codes.spindle_off),
            Event::Coolant { mode } => self.block(match mode {
                Coolant::Flood => &codes.coolant_flood,
                Coolant::Mist => &codes.coolant_mist,
                Coolant::Off => &codes.coolant_off,
            }),
            Event::ProgramStop { optional } => self.block(if *optional {
                &codes.optional_stop
            } else {
                &codes.stop
            }),
            Event::Comment { text } => self.comment(text),
        }
    }

    fn finish(self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_formatted_safely() {
        assert_eq!(number(10.0, 3, true, true), "10.");
        assert_eq!(number(10.0, 3, true, false), "10");
        assert_eq!(number(10.25, 3, true, true), "10.25");
        assert_eq!(number(-0.0001, 3, true, true), "0.");
        assert_eq!(number(-0.0001, 3, false, false), "0.000");
        assert_eq!(number(-2.5, 3, false, false), "-2.500");
        assert_eq!(number(12000.0, 0, true, false), "12000");
    }

    #[test]
    fn placeholders_are_found_and_checked() {
        assert_eq!(placeholders("M3 S{rpm}").unwrap(), vec!["rpm"]);
        assert!(placeholders("M3 S{rpm").is_err());
        assert!(placeholders("M3 S}").is_err());
    }
}
