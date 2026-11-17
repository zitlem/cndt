// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading posted G-code back into a toolpath, so the program a post wrote
//! can be simulated like the toolpath it came from (docs/safety.md, "Known
//! gap"). It reads the words the shipped posts write, Fanuc-style, and
//! refuses anything else with the line and the reason. It never guesses:
//! a program it cannot read is reported as not read back.

use cndt_cam_core::{
    ArcDirection, Coolant, Cycle, CycleKind, Event, Move, Operation, Program, SpindleDirection,
    Step,
};
use cndt_geom::Point3;

/// Where an axis is unknown after homing, the tool is taken to be this far
/// above program zero: out of reach of any stock, so the first rapid down
/// is through air, as it is on the machine.
pub const UNKNOWN_HEIGHT: f64 = 1000.0;

/// Why a program could not be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError {
    /// The line, counting from 1.
    pub line: usize,
    /// What could not be read.
    pub reason: String,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ReadError {}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    None,
    Rapid,
    Linear,
    Arc(ArcDirection),
    Cycle,
}

struct Reader {
    steps: Vec<Step>,
    work_offset: Option<u8>,
    /// Known axes; `None` after homing until set again.
    at: [Option<f64>; 3],
    mode: Mode,
    metric: bool,
    feed: Option<f64>,
    rpm: Option<f64>,
    /// The cycle being collected: its kind code, clearance, bottom, feed
    /// and holes so far.
    cycle: Option<(String, Cycle)>,
    peck: Option<f64>,
    /// Whether a tool number has been read: a cut before one cannot be
    /// simulated with the right tool.
    tool: bool,
}

/// Reads a posted program back into a toolpath: its moves, cycles, tool
/// changes, spindle and coolant, homing, table turns and work offsets, in
/// program coordinates and millimetres.
///
/// # Errors
///
/// A [`ReadError`] naming the first line it cannot read: incremental
/// moves, arcs given by radius, `G99` returns, or any word it does not
/// know.
pub fn read_back(gcode: &str) -> Result<Program, ReadError> {
    let mut reader = Reader {
        steps: Vec::new(),
        work_offset: None,
        at: [None; 3],
        mode: Mode::None,
        metric: true,
        feed: None,
        rpm: None,
        cycle: None,
        peck: None,
        tool: false,
    };
    for (number, raw) in gcode.lines().enumerate() {
        reader.line(raw).map_err(|reason| ReadError {
            line: number + 1,
            reason,
        })?;
    }
    reader.finish_cycle();
    Ok(Program {
        name: String::from("read back"),
        number: 0,
        work_offset: reader.work_offset.unwrap_or(1),
        operations: vec![Operation {
            name: String::from("read back"),
            steps: reader.steps,
        }],
    })
}

/// The words of a line, comments, line numbers and blocks such as `%`
/// left out.
fn words(raw: &str) -> Result<Vec<(char, String)>, String> {
    let mut text = String::new();
    let mut depth = 0_u32;
    for c in raw.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => break,
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let text = text.trim();
    if text.is_empty() || text == "%" || text.starts_with('O') {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut chars = text.chars().filter(|c| !c.is_whitespace()).peekable();
    while let Some(letter) = chars.next() {
        if !letter.is_ascii_alphabetic() {
            return Err(format!("unexpected {letter:?}"));
        }
        let mut value = String::new();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+' {
                value.push(*c);
                chars.next();
            } else {
                break;
            }
        }
        if value.is_empty() {
            return Err(format!("{letter} has no value"));
        }
        out.push((letter.to_ascii_uppercase(), value));
    }
    Ok(out.into_iter().filter(|(l, _)| *l != 'N').collect())
}

fn number(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{value:?} is not a number"))
}

impl Reader {
    fn scale(&self) -> f64 {
        if self.metric { 1.0 } else { 25.4 }
    }

    fn line(&mut self, raw: &str) -> Result<(), String> {
        let words = words(raw)?;
        if words.is_empty() {
            return Ok(());
        }
        let mut axes: [Option<f64>; 3] = [None; 3];
        let (mut i, mut j) = (None, None);
        let (mut r, mut q, mut k, mut h) = (None, None, None, None);
        let mut f = None;
        let mut s = None;
        let mut t = None;
        let mut rotary = None;
        let mut gs = Vec::new();
        let mut ms = Vec::new();
        for (letter, value) in &words {
            let v = || number(value);
            match letter {
                'G' => gs.push(value.clone()),
                'M' => ms.push(v()?),
                'X' => axes[0] = Some(v()? * self.scale()),
                'Y' => axes[1] = Some(v()? * self.scale()),
                'Z' => axes[2] = Some(v()? * self.scale()),
                'I' => i = Some(v()? * self.scale()),
                'J' => j = Some(v()? * self.scale()),
                'R' => r = Some(v()? * self.scale()),
                'Q' => q = Some(v()? * self.scale()),
                'K' => k = Some(v()? * self.scale()),
                'F' => f = Some(v()? * self.scale()),
                'S' => s = Some(v()?),
                'T' => t = Some(v()?),
                'H' | 'D' | 'P' => h = Some(v()?),
                'A' | 'B' | 'C' => rotary = Some((*letter, v()?)),
                other => return Err(format!("unknown word {other}")),
            }
        }
        let g = |code: &str| gs.iter().any(|x| trim_code(x) == code);
        // H (length offset), D and P (dwell) change no position.
        let _ = h;

        // Machine coordinates, homing and incremental returns: the tool
        // goes to a safe place and its position is unknown.
        if g("53") || g("28") || g("30") {
            self.finish_cycle();
            if !matches!(self.steps.last(), Some(Step::Event(Event::Home))) {
                self.steps.push(Step::Event(Event::Home));
            }
            self.at = [None; 3];
            self.mode = Mode::None;
            return self.spindle_and_coolant(&ms, s);
        }
        for code in &gs {
            match trim_code(code) {
                // Plane, cutter and length compensation, feed mode, path
                // blending (LinuxCNC G64 P), cancel, return and absolute:
                // none moves the tool.
                "17" | "40" | "49" | "94" | "43" | "80" | "98" | "90" | "64" | "61" => {}
                code @ ("0" | "1" | "2" | "3") => {
                    self.finish_cycle();
                    self.mode = match code {
                        "0" => Mode::Rapid,
                        "1" => Mode::Linear,
                        "2" => Mode::Arc(ArcDirection::Clockwise),
                        _ => Mode::Arc(ArcDirection::CounterClockwise),
                    };
                }
                "20" => self.metric = false,
                "21" => self.metric = true,
                "91" => {
                    // Only a bare G91 before a G90 is harmless; any move in
                    // it is refused below.
                    if axes.iter().any(Option::is_some) {
                        return Err(String::from("incremental moves are not read back"));
                    }
                }
                "99" => return Err(String::from("G99 returns are not read back")),
                "4" => {}
                code @ ("54" | "55" | "56" | "57" | "58" | "59") => {
                    self.finish_cycle();
                    let offset = code.parse::<u8>().map_err(|e| e.to_string())? - 53;
                    if self.work_offset.is_none() {
                        self.work_offset = Some(offset);
                    } else {
                        self.steps.push(Step::Event(Event::WorkOffset { offset }));
                    }
                    self.at = [None; 3];
                }
                code @ ("81" | "82" | "83" | "84" | "84.2" | "85" | "74" | "73") => {
                    self.finish_cycle();
                    self.mode = Mode::Cycle;
                    self.cycle = Some((
                        code.to_owned(),
                        Cycle {
                            kind: CycleKind::Drill,
                            holes: Vec::new(),
                            clearance: 0.0,
                            bottom: 0.0,
                            feed: 0.0,
                        },
                    ));
                }
                "33.1" => {}
                other => return Err(format!("unknown code G{other}")),
            }
        }
        if g("80") {
            self.finish_cycle();
            self.mode = Mode::None;
        }
        if let Some(t) = t {
            if !ms.iter().any(|m| (*m - 6.0).abs() < 1e-9) {
                return Err(String::from("a tool number without M6 is not read back"));
            }
            self.finish_cycle();
            self.tool = true;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            self.steps.push(Step::Event(Event::ToolChange {
                number: t as u32,
                description: String::new(),
            }));
        }
        if let Some(feed) = f {
            self.feed = Some(feed);
        }
        if let Some(rpm) = s {
            self.rpm = Some(rpm);
        }
        if let Some((axis, angle)) = rotary {
            self.finish_cycle();
            if axes.iter().any(Option::is_some) {
                return Err(String::from("a table turn with a move is not read back"));
            }
            self.steps.push(Step::Event(Event::Index {
                axis: axis.to_string(),
                angle,
            }));
            return self.spindle_and_coolant(&ms, None);
        }
        // M codes before the motion of the same line: spindle and coolant
        // start before a move written with them.
        self.spindle_and_coolant(&ms, s)?;

        let cuts = g("33.1")
            || (axes.iter().any(Option::is_some)
                && matches!(self.mode, Mode::Linear | Mode::Arc(_) | Mode::Cycle));
        if cuts && !self.tool {
            return Err(String::from(
                "a cut before any tool number; this post's tool changes cannot be read back",
            ));
        }
        if g("33.1") {
            return self.tap_move(axes, k);
        }
        if axes.iter().any(Option::is_some) {
            match self.mode {
                Mode::None => return Err(String::from("a move with no motion mode")),
                Mode::Rapid => {
                    let to = self.target(axes, true)?;
                    self.steps.push(Step::Move(Move::Rapid { to }));
                }
                Mode::Linear => {
                    let to = self.target(axes, false)?;
                    let feed = self.feed.ok_or("a feed move with no feed rate")?;
                    self.steps.push(Step::Move(Move::Linear { to, feed }));
                }
                Mode::Arc(direction) => {
                    if r.is_some() {
                        return Err(String::from("arcs given by radius are not read back"));
                    }
                    let from = self.known()?;
                    let to = self.target(axes, false)?;
                    let center =
                        Point3::new(from.x + i.unwrap_or(0.0), from.y + j.unwrap_or(0.0), from.z);
                    let feed = self.feed.ok_or("an arc with no feed rate")?;
                    self.steps.push(Step::Move(Move::Arc {
                        to,
                        center,
                        direction,
                        feed,
                    }));
                }
                Mode::Cycle => self.hole(axes, r, q)?,
            }
        }
        Ok(())
    }

    fn spindle_and_coolant(&mut self, ms: &[f64], s: Option<f64>) -> Result<(), String> {
        for m in ms {
            #[allow(clippy::cast_possible_truncation)]
            let code = (m * 10.0).round() as i64;
            let event = match code {
                30 | 40 => {
                    let rpm = s.or(self.rpm).ok_or("the spindle starts with no speed")?;
                    self.rpm = Some(rpm);
                    Some(Event::SpindleOn {
                        rpm,
                        direction: if code == 30 {
                            SpindleDirection::Clockwise
                        } else {
                            SpindleDirection::CounterClockwise
                        },
                    })
                }
                50 => Some(Event::SpindleOff),
                70 => Some(Event::Coolant {
                    mode: Coolant::Mist,
                }),
                80 => Some(Event::Coolant {
                    mode: Coolant::Flood,
                }),
                90 => Some(Event::Coolant { mode: Coolant::Off }),
                0 => Some(Event::ProgramStop { optional: false }),
                10 => Some(Event::ProgramStop { optional: true }),
                // Tool change (with its T word), rigid tap mode, program end.
                60 | 290 | 300 | 20 => None,
                other => return Err(format!("unknown code M{}", other as f64 / 10.0)),
            };
            if let Some(event) = event {
                self.finish_cycle();
                self.steps.push(Step::Event(event));
            }
        }
        Ok(())
    }

    /// The point a move goes to; a rapid may start from an unknown height,
    /// which is then taken as far above everything.
    fn target(&mut self, axes: [Option<f64>; 3], rapid: bool) -> Result<Point3, String> {
        let mut out = [0.0; 3];
        for axis in 0..3 {
            out[axis] = match (axes[axis], self.at[axis]) {
                (Some(v), _) | (None, Some(v)) => v,
                (None, None) if rapid && axis == 2 => UNKNOWN_HEIGHT,
                (None, None) => {
                    return Err(format!(
                        "{} is unknown here; a move must set it first",
                        ["X", "Y", "Z"][axis]
                    ));
                }
            };
        }
        // Once moved, an axis given is known; an unknown Z stays unknown
        // until set, so the next rapid down starts from above.
        for axis in 0..3 {
            if axes[axis].is_some() || self.at[axis].is_some() {
                self.at[axis] = Some(out[axis]);
            }
        }
        Ok(Point3::new(out[0], out[1], out[2]))
    }

    fn known(&self) -> Result<Point3, String> {
        match self.at {
            [Some(x), Some(y), Some(z)] => Ok(Point3::new(x, y, z)),
            _ => Err(String::from("the position is unknown here")),
        }
    }

    fn hole(
        &mut self,
        axes: [Option<f64>; 3],
        r: Option<f64>,
        q: Option<f64>,
    ) -> Result<(), String> {
        let start = self.known()?;
        let Some((code, cycle)) = self.cycle.as_mut() else {
            return Err(String::from("a hole with no cycle"));
        };
        if cycle.holes.is_empty() {
            cycle.bottom = axes[2].ok_or("a cycle with no depth")?;
            cycle.clearance = r.ok_or("a cycle with no R plane")?;
            cycle.feed = self.feed.ok_or("a cycle with no feed rate")?;
            if let Some(q) = q {
                self.peck = Some(q);
            }
            cycle.kind = match code.as_str() {
                "81" | "82" => CycleKind::Drill,
                "83" | "73" => CycleKind::Peck {
                    step: self.peck.ok_or("a peck cycle with no Q")?,
                },
                "84" | "84.2" | "74" => CycleKind::Tap {
                    pitch: cycle.feed / self.rpm.ok_or("a tap cycle with no spindle speed")?,
                },
                _ => CycleKind::Ream,
            };
        } else if axes[2].is_some() || r.is_some() {
            return Err(String::from("a cycle that changes depth between holes"));
        }
        let x = axes[0].unwrap_or(start.x);
        let y = axes[1].unwrap_or(start.y);
        cycle.holes.push(Point3::new(x, y, 0.0));
        // The tool ends over the hole, back at the starting height (G98).
        self.at = [Some(x), Some(y), Some(start.z)];
        Ok(())
    }

    /// A spindle-synchronised tap (G33.1): in to the depth and back out to
    /// where it started, which is a one-hole tapping cycle from there.
    fn tap_move(&mut self, axes: [Option<f64>; 3], k: Option<f64>) -> Result<(), String> {
        let from = self.known()?;
        if axes[0].is_some() || axes[1].is_some() {
            return Err(String::from("a synchronised tap that moves sideways"));
        }
        let bottom = axes[2].ok_or("a synchronised tap with no depth")?;
        let pitch = k.ok_or("a synchronised tap with no pitch")?;
        let rpm = self.rpm.ok_or("a synchronised tap with no spindle speed")?;
        self.finish_cycle();
        self.steps.push(Step::Move(Move::Cycle(Cycle {
            kind: CycleKind::Tap { pitch },
            holes: vec![Point3::new(from.x, from.y, 0.0)],
            clearance: from.z,
            bottom,
            feed: pitch * rpm,
        })));
        Ok(())
    }

    /// Ends the cycle being collected, keeping it if it has holes.
    fn finish_cycle(&mut self) {
        if let Some((_, cycle)) = self.cycle.take()
            && !cycle.holes.is_empty()
        {
            self.steps.push(Step::Move(Move::Cycle(cycle)));
        }
    }
}

/// `G0` and `G00` are the same code.
fn trim_code(code: &str) -> &str {
    let trimmed = code.trim_start_matches('0');
    if trimmed.is_empty() || trimmed.starts_with('.') {
        "0"
    } else {
        trimmed
    }
}
