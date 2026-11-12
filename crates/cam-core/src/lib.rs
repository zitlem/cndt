// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 2 CAM core: the neutral toolpath format.
//!
//! Every machining process (milling, turning, wire EDM) produces a
//! [`Program`] in this one format. Simulation, the post-processor and the
//! viewport all consume it, and a post never sees model geometry
//! (architecture rules 6 and 7). Nothing in here is specific to one process.
//!
//! Lengths are millimetres, feeds millimetres per minute, spindle speeds
//! revolutions per minute.

use std::fmt;

use cndt_geom::Point3;
use serde::{Deserialize, Serialize};

/// Which way an arc turns, seen from +Z looking down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArcDirection {
    /// Clockwise.
    Clockwise,
    /// Counter-clockwise.
    CounterClockwise,
}

/// A tool motion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Move {
    /// Move at full speed. Never used for cutting.
    Rapid {
        /// The end point.
        to: Point3,
    },
    /// Cut in a straight line.
    Linear {
        /// The end point.
        to: Point3,
        /// Feed rate in mm/min.
        feed: f64,
    },
    /// Cut along an arc in the XY plane. Z may change, giving a helix.
    Arc {
        /// The end point.
        to: Point3,
        /// The arc's centre. Only X and Y are used.
        center: Point3,
        /// Which way it turns.
        direction: ArcDirection,
        /// Feed rate in mm/min.
        feed: f64,
    },
    /// Stay put.
    Dwell {
        /// How long, in seconds.
        seconds: f64,
    },
}

/// Which way the spindle turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpindleDirection {
    /// Clockwise, for right-hand tools.
    Clockwise,
    /// Counter-clockwise.
    CounterClockwise,
}

/// Coolant modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coolant {
    /// Flood coolant.
    Flood,
    /// Mist coolant.
    Mist,
    /// Coolant off.
    Off,
}

/// Anything in a toolpath that is not a motion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Load a tool.
    ToolChange {
        /// The tool's number in the machine's magazine.
        number: u32,
        /// A short description for the program listing.
        description: String,
    },
    /// Start the spindle.
    SpindleOn {
        /// Speed in rpm.
        rpm: f64,
        /// Direction of rotation.
        direction: SpindleDirection,
    },
    /// Stop the spindle.
    SpindleOff,
    /// Change coolant.
    Coolant {
        /// The new mode.
        mode: Coolant,
    },
    /// Stop the program until the operator restarts it.
    ProgramStop {
        /// `true` for an optional stop the operator can switch off.
        optional: bool,
    },
    /// A note for the operator in the program listing.
    Comment {
        /// The text.
        text: String,
    },
}

/// One entry in a toolpath.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// A motion.
    Move(Move),
    /// Anything else.
    Event(Event),
}

/// One machining operation's toolpath, such as a pocket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    /// The operation's name, used in comments and error messages.
    pub name: String,
    /// What it does, in order.
    pub steps: Vec<Step>,
}

/// A complete program for one setup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// A name for the listing.
    pub name: String,
    /// The program number, where the controller uses one.
    pub number: u32,
    /// The work offset: 1 is G54 through 6 for G59.
    pub work_offset: u8,
    /// The operations, in machining order.
    pub operations: Vec<Operation>,
}

/// Where in a program a problem was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The operation's name.
    pub operation: String,
    /// The step's position in the operation, counting from 1.
    pub step: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, step {}", self.operation, self.step)
    }
}

/// Why a program is not safe to post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolpathError {
    /// The work offset is outside 1 to 6.
    BadWorkOffset(u8),
    /// A number is infinite or not a number.
    NotFinite(Location),
    /// A feed rate is zero or negative.
    BadFeed(Location),
    /// A spindle speed is zero or negative.
    BadSpindleSpeed(Location),
    /// A dwell time is negative.
    BadDwell(Location),
    /// A motion comes before the first rapid, so the start point is unknown.
    UnknownStart(Location),
    /// A cutting move comes before any tool is loaded.
    NoTool(Location),
    /// A cutting move comes while the spindle is off.
    SpindleOff(Location),
    /// An arc's start and end are not the same distance from its centre, or
    /// its radius is zero.
    BadArc(Location),
}

impl fmt::Display for ToolpathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadWorkOffset(offset) => {
                write!(f, "work offset {offset} is not between 1 (G54) and 6 (G59)")
            }
            Self::NotFinite(at) => write!(f, "{at}: a value is not a finite number"),
            Self::BadFeed(at) => write!(f, "{at}: the feed rate must be above zero"),
            Self::BadSpindleSpeed(at) => write!(f, "{at}: the spindle speed must be above zero"),
            Self::BadDwell(at) => write!(f, "{at}: a dwell cannot be negative"),
            Self::UnknownStart(at) => {
                write!(
                    f,
                    "{at}: the tool moves before its position is known; start with a rapid"
                )
            }
            Self::NoTool(at) => write!(f, "{at}: cutting before any tool is loaded"),
            Self::SpindleOff(at) => write!(f, "{at}: cutting with the spindle off"),
            Self::BadArc(at) => write!(
                f,
                "{at}: the arc's start and end are not the same distance from its centre"
            ),
        }
    }
}

impl std::error::Error for ToolpathError {}

/// How far an arc's start and end radius may differ, in millimetres.
pub const ARC_TOLERANCE: f64 = 0.001;

impl Program {
    /// Checks that the program is safe to post: every number finite, feeds
    /// and speeds positive, no motion before the position is known, no cutting
    /// without a tool or with the spindle off, and every arc consistent.
    ///
    /// # Errors
    ///
    /// Returns the first problem found, with its location.
    pub fn validate(&self) -> Result<(), ToolpathError> {
        if !(1..=6).contains(&self.work_offset) {
            return Err(ToolpathError::BadWorkOffset(self.work_offset));
        }
        let mut position: Option<Point3> = None;
        let mut tool_loaded = false;
        let mut spindle_on = false;

        for operation in &self.operations {
            for (index, step) in operation.steps.iter().enumerate() {
                let at = || Location {
                    operation: operation.name.clone(),
                    step: index + 1,
                };
                match step {
                    Step::Event(Event::ToolChange { .. }) => {
                        tool_loaded = true;
                        // Machines stop the spindle for a tool change.
                        spindle_on = false;
                    }
                    Step::Event(Event::SpindleOn { rpm, .. }) => {
                        if !rpm.is_finite() {
                            return Err(ToolpathError::NotFinite(at()));
                        }
                        if *rpm <= 0.0 {
                            return Err(ToolpathError::BadSpindleSpeed(at()));
                        }
                        spindle_on = true;
                    }
                    Step::Event(Event::SpindleOff) => spindle_on = false,
                    Step::Event(_) => {}
                    Step::Move(Move::Dwell { seconds }) => {
                        if !seconds.is_finite() {
                            return Err(ToolpathError::NotFinite(at()));
                        }
                        if *seconds < 0.0 {
                            return Err(ToolpathError::BadDwell(at()));
                        }
                    }
                    Step::Move(Move::Rapid { to }) => {
                        check_point(to, &at)?;
                        position = Some(*to);
                    }
                    Step::Move(Move::Linear { to, feed }) => {
                        check_cut(*feed, position, tool_loaded, spindle_on, &at)?;
                        check_point(to, &at)?;
                        position = Some(*to);
                    }
                    Step::Move(Move::Arc {
                        to, center, feed, ..
                    }) => {
                        let start = check_cut(*feed, position, tool_loaded, spindle_on, &at)?;
                        check_point(to, &at)?;
                        check_point(center, &at)?;
                        let start_radius = (start.x - center.x).hypot(start.y - center.y);
                        let end_radius = (to.x - center.x).hypot(to.y - center.y);
                        if start_radius < ARC_TOLERANCE
                            || (start_radius - end_radius).abs() > ARC_TOLERANCE
                        {
                            return Err(ToolpathError::BadArc(at()));
                        }
                        position = Some(*to);
                    }
                }
            }
        }
        Ok(())
    }
}

fn check_point(point: &Point3, at: &impl Fn() -> Location) -> Result<(), ToolpathError> {
    if point.x.is_finite() && point.y.is_finite() && point.z.is_finite() {
        Ok(())
    } else {
        Err(ToolpathError::NotFinite(at()))
    }
}

/// Checks the preconditions of any cutting move and returns its start point.
fn check_cut(
    feed: f64,
    position: Option<Point3>,
    tool_loaded: bool,
    spindle_on: bool,
    at: &impl Fn() -> Location,
) -> Result<Point3, ToolpathError> {
    if !feed.is_finite() {
        return Err(ToolpathError::NotFinite(at()));
    }
    if feed <= 0.0 {
        return Err(ToolpathError::BadFeed(at()));
    }
    let start = position.ok_or_else(|| ToolpathError::UnknownStart(at()))?;
    if !tool_loaded {
        return Err(ToolpathError::NoTool(at()));
    }
    if !spindle_on {
        return Err(ToolpathError::SpindleOff(at()));
    }
    Ok(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    fn program(steps: Vec<Step>) -> Program {
        Program {
            name: String::from("test"),
            number: 1,
            work_offset: 1,
            operations: vec![Operation {
                name: String::from("op"),
                steps,
            }],
        }
    }

    fn ready() -> Vec<Step> {
        vec![
            Step::Event(Event::ToolChange {
                number: 1,
                description: String::from("6 mm end mill"),
            }),
            Step::Event(Event::SpindleOn {
                rpm: 10_000.0,
                direction: SpindleDirection::Clockwise,
            }),
            Step::Move(Move::Rapid {
                to: p(10.0, 0.0, 5.0),
            }),
        ]
    }

    fn with(extra: Step) -> Program {
        let mut steps = ready();
        steps.push(extra);
        program(steps)
    }

    #[test]
    fn a_sound_program_passes() {
        let mut steps = ready();
        steps.push(Step::Move(Move::Linear {
            to: p(10.0, 0.0, -1.0),
            feed: 300.0,
        }));
        steps.push(Step::Move(Move::Arc {
            to: p(-10.0, 0.0, -1.0),
            center: p(0.0, 0.0, -1.0),
            direction: ArcDirection::CounterClockwise,
            feed: 600.0,
        }));
        assert_eq!(program(steps).validate(), Ok(()));
    }

    #[test]
    fn unsafe_programs_are_refused_with_a_location() {
        let cut = |feed| {
            Step::Move(Move::Linear {
                to: p(0.0, 0.0, 0.0),
                feed,
            })
        };
        assert!(matches!(
            with(cut(0.0)).validate(),
            Err(ToolpathError::BadFeed(_))
        ));
        assert!(matches!(
            with(cut(f64::NAN)).validate(),
            Err(ToolpathError::NotFinite(_))
        ));

        let no_start = program(vec![ready()[0].clone(), ready()[1].clone(), cut(100.0)]);
        assert!(matches!(
            no_start.validate(),
            Err(ToolpathError::UnknownStart(_))
        ));

        let no_tool = program(vec![
            Step::Move(Move::Rapid {
                to: p(0.0, 0.0, 5.0),
            }),
            cut(100.0),
        ]);
        assert!(matches!(no_tool.validate(), Err(ToolpathError::NoTool(_))));

        let mut stopped = ready();
        stopped.push(Step::Event(Event::SpindleOff));
        stopped.push(cut(100.0));
        let error = program(stopped).validate().unwrap_err();
        assert!(matches!(error, ToolpathError::SpindleOff(_)));
        assert_eq!(
            error.to_string(),
            "op, step 5: cutting with the spindle off"
        );

        let bad_arc = with(Step::Move(Move::Arc {
            to: p(0.0, 20.0, 5.0),
            center: p(0.0, 0.0, 5.0),
            direction: ArcDirection::Clockwise,
            feed: 100.0,
        }));
        assert!(matches!(bad_arc.validate(), Err(ToolpathError::BadArc(_))));

        let mut offset = with(cut(100.0));
        offset.work_offset = 7;
        assert_eq!(offset.validate(), Err(ToolpathError::BadWorkOffset(7)));
    }

    #[test]
    fn a_tool_change_stops_the_spindle() {
        let mut steps = ready();
        steps.push(Step::Event(Event::ToolChange {
            number: 2,
            description: String::from("drill"),
        }));
        steps.push(Step::Move(Move::Linear {
            to: p(0.0, 0.0, 0.0),
            feed: 100.0,
        }));
        assert!(matches!(
            program(steps).validate(),
            Err(ToolpathError::SpindleOff(_))
        ));
    }
}
