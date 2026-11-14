// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machine definitions (decision 0015): what a machine can do, read from a
//! plain-text file in `machines/`, and the check that a program stays
//! within it.

use std::fmt;

use cndt_geom::Point3;
use serde::Deserialize;

use crate::{CycleKind, Event, Move, Program, Step};

/// Which way the spindle points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// The spindle points down at the table.
    Vertical,
    /// The spindle points sideways at a pallet or tombstone.
    Horizontal,
}

/// The `[machine]` section.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// A short id, the file's name without `.toml`.
    pub id: String,
    /// A name for people.
    pub name: String,
    /// Which way the spindle points.
    pub layout: Layout,
    /// The post this machine is normally used with.
    pub post: String,
    /// Whether the values have been checked on the machine itself.
    pub verified: bool,
}

/// How far each linear axis can move, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Travel {
    /// X travel.
    pub x: f64,
    /// Y travel.
    pub y: f64,
    /// Z travel.
    pub z: f64,
}

/// The `[spindle]` section.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spindle {
    /// The fastest speed, in rpm.
    pub max_rpm: f64,
    /// The tool holder taper, such as `CAT40`.
    pub taper: String,
    /// Whether it can tap rigidly (synchronised with the feed).
    pub rigid_tapping: bool,
}

/// The `[magazine]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Magazine {
    /// The highest tool number.
    pub pockets: u32,
}

/// A rotary axis. Described, not yet used by any operation.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotary {
    /// Its letter, such as `B`.
    pub axis: String,
    /// The linear axis it turns around: `X`, `Y` or `Z`.
    pub around: String,
    /// The smallest step in degrees; 0 for a fully interpolating axis.
    pub index: f64,
}

/// A machine definition.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    /// Who it is.
    pub machine: Identity,
    /// Axis travel.
    pub travel: Travel,
    /// The spindle.
    pub spindle: Spindle,
    /// The tool magazine.
    pub magazine: Magazine,
    /// Rotary axes, if any.
    #[serde(default)]
    pub rotary: Vec<Rotary>,
}

/// Why a machine definition or a program does not fit.
#[derive(Debug, Clone, PartialEq)]
pub enum MachineError {
    /// The definition could not be read; says why.
    Definition(String),
    /// The program asks for something the machine cannot do; says what.
    Limit(String),
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition(reason) => write!(f, "bad machine definition: {reason}"),
            Self::Limit(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for MachineError {}

impl Machine {
    /// Reads a definition from TOML. Unknown fields are refused, so a typo
    /// cannot silently drop a limit.
    ///
    /// # Errors
    ///
    /// [`MachineError::Definition`] when the text is not a valid
    /// definition or a value is out of range.
    pub fn from_toml(text: &str) -> Result<Self, MachineError> {
        let machine: Self =
            toml::from_str(text).map_err(|e| MachineError::Definition(e.to_string()))?;
        let bad = |what: &str| Err(MachineError::Definition(what.to_owned()));
        let t = machine.travel;
        if !([t.x, t.y, t.z].iter().all(|v| v.is_finite() && *v > 0.0)) {
            return bad("every travel must be more than zero");
        }
        if !(machine.spindle.max_rpm.is_finite() && machine.spindle.max_rpm > 0.0) {
            return bad("the spindle's max_rpm must be more than zero");
        }
        if machine.magazine.pockets == 0 {
            return bad("the magazine must have at least one pocket");
        }
        for rotary in &machine.rotary {
            if !matches!(rotary.around.as_str(), "X" | "Y" | "Z") {
                return bad("a rotary axis must turn around X, Y or Z");
            }
            if !(rotary.index.is_finite() && rotary.index >= 0.0) {
                return bad("a rotary index step cannot be negative");
            }
        }
        Ok(machine)
    }

    /// Checks that a program stays within this machine: spindle speed,
    /// tool numbers, rigid tapping, and the program's extent along each
    /// axis against that axis's travel. The extent check cannot tell where
    /// the part sits on the machine, only that the program could fit.
    ///
    /// # Errors
    ///
    /// [`MachineError::Limit`] naming the first operation that breaks a
    /// limit and how.
    pub fn check(&self, program: &Program) -> Result<(), MachineError> {
        let name = &self.machine.name;
        let mut extent = Extent::default();
        let mut position: Option<Point3> = None;
        for operation in &program.operations {
            let limit =
                |what: String| Err(MachineError::Limit(format!("{}: {what}", operation.name)));
            for step in &operation.steps {
                match step {
                    Step::Event(Event::SpindleOn { rpm, .. }) if *rpm > self.spindle.max_rpm => {
                        return limit(format!(
                            "{rpm} rpm is faster than the {name}'s {} rpm",
                            self.spindle.max_rpm
                        ));
                    }
                    Step::Event(Event::ToolChange { number, .. })
                        if *number == 0 || *number > self.magazine.pockets =>
                    {
                        return limit(format!(
                            "tool {number} is not in the {name}'s magazine of {} pockets",
                            self.magazine.pockets
                        ));
                    }
                    Step::Event(Event::Index { axis, angle }) => {
                        if let Err(reason) = self.check_index(axis, *angle) {
                            return limit(reason);
                        }
                        // Each side has its own work offset: measure each alone.
                        self.check_extent(&extent)?;
                        extent = Extent::default();
                        position = None;
                    }
                    Step::Event(Event::WorkOffset { .. }) => {
                        self.check_extent(&extent)?;
                        extent = Extent::default();
                        position = None;
                    }
                    Step::Move(Move::Rapid { to } | Move::Linear { to, .. }) => {
                        extent.add(*to);
                        position = Some(*to);
                    }
                    Step::Move(Move::Arc { to, center, .. }) => {
                        // The whole circle: never smaller than the arc.
                        let radius = (to.x - center.x).hypot(to.y - center.y);
                        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                            extent.add(Point3::new(
                                center.x + dx * radius,
                                center.y + dy * radius,
                                to.z,
                            ));
                        }
                        extent.add(*to);
                        position = Some(*to);
                    }
                    Step::Move(Move::Cycle(cycle)) => {
                        if matches!(cycle.kind, CycleKind::Tap { .. })
                            && !self.spindle.rigid_tapping
                        {
                            return limit(format!("the {name} cannot tap rigidly"));
                        }
                        let top = position.map_or(cycle.clearance, |p| p.z);
                        for hole in &cycle.holes {
                            extent.add(Point3::new(hole.x, hole.y, top));
                            extent.add(Point3::new(hole.x, hole.y, cycle.bottom));
                        }
                        if let Some(last) = cycle.holes.last() {
                            position = Some(Point3::new(last.x, last.y, top));
                        }
                    }
                    _ => {}
                }
            }
        }
        self.check_extent(&extent)
    }

    /// The program's extent along each axis against that axis's travel.
    fn check_extent(&self, extent: &Extent) -> Result<(), MachineError> {
        let travel = self.travel;
        for (axis, size, limit) in [
            ("X", extent.size(|p| p.x), travel.x),
            ("Y", extent.size(|p| p.y), travel.y),
            ("Z", extent.size(|p| p.z), travel.z),
        ] {
            if size > limit {
                return Err(MachineError::Limit(format!(
                    "the program spans {size:.3} mm in {axis}, more than the {}'s {limit} mm of travel",
                    self.machine.name
                )));
            }
        }
        Ok(())
    }

    /// Checks a turn of a rotary axis to `angle` degrees.
    fn check_index(&self, axis: &str, angle: f64) -> Result<(), String> {
        let name = &self.machine.name;
        let Some(rotary) = self.rotary.iter().find(|r| r.axis == axis) else {
            return Err(format!("the {name} has no {axis} axis"));
        };
        if rotary.index > 0.0 {
            let steps = angle / rotary.index;
            if (steps - steps.round()).abs() > 1e-6 {
                return Err(format!(
                    "{axis}{angle} is not a whole number of the {name}'s {}° steps",
                    rotary.index
                ));
            }
        }
        Ok(())
    }
}

/// The smallest box around a set of points.
#[derive(Default)]
struct Extent {
    bounds: Option<(Point3, Point3)>,
}

impl Extent {
    fn add(&mut self, p: Point3) {
        self.bounds = Some(match self.bounds {
            None => (p, p),
            Some((lo, hi)) => (
                Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
            ),
        });
    }

    fn size(&self, axis: impl Fn(&Point3) -> f64) -> f64 {
        self.bounds.map_or(0.0, |(lo, hi)| axis(&hi) - axis(&lo))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{Cycle, Operation, SpindleDirection};

    const MAZAK: &str = include_str!("../../../machines/mazak-hcn-5000.toml");

    fn program(steps: Vec<Step>) -> Program {
        Program {
            name: "test".into(),
            number: 1,
            work_offset: 1,
            operations: vec![Operation {
                name: "Op".into(),
                steps,
            }],
        }
    }

    fn tool(number: u32) -> Step {
        Step::Event(Event::ToolChange {
            number,
            description: String::new(),
        })
    }

    fn spindle(rpm: f64) -> Step {
        Step::Event(Event::SpindleOn {
            rpm,
            direction: SpindleDirection::Clockwise,
        })
    }

    fn rapid(x: f64, y: f64, z: f64) -> Step {
        Step::Move(Move::Rapid {
            to: Point3::new(x, y, z),
        })
    }

    #[test]
    fn the_shipped_mazak_loads() {
        let machine = Machine::from_toml(MAZAK).unwrap();
        assert_eq!(machine.machine.layout, Layout::Horizontal);
        assert_eq!(machine.machine.post, "mazak-smoothg");
        assert!(!machine.machine.verified);
        assert_eq!(machine.rotary.len(), 1);
    }

    #[test]
    fn a_typo_fails_to_load() {
        let text = MAZAK.replace("max_rpm", "max_rmp");
        assert!(matches!(
            Machine::from_toml(&text),
            Err(MachineError::Definition(_))
        ));
        let text = MAZAK.replace("pockets = 40", "pockets = 0");
        assert!(Machine::from_toml(&text).is_err());
    }

    #[test]
    fn a_program_within_the_limits_passes() {
        let machine = Machine::from_toml(MAZAK).unwrap();
        let ok = program(vec![
            tool(14),
            spindle(12000.0),
            rapid(0.0, 0.0, 50.0),
            rapid(700.0, 700.0, -500.0),
        ]);
        machine.check(&ok).unwrap();
    }

    #[test]
    fn limits_are_enforced() {
        let machine = Machine::from_toml(MAZAK).unwrap();
        let fast = program(vec![tool(1), spindle(12000.5)]);
        let error = machine.check(&fast).unwrap_err().to_string();
        assert!(error.contains("faster than"), "{error}");

        let missing = program(vec![tool(41)]);
        let error = machine.check(&missing).unwrap_err().to_string();
        assert!(error.contains("tool 41"), "{error}");
        assert!(machine.check(&program(vec![tool(0)])).is_err());

        let wide = program(vec![rapid(0.0, 0.0, 5.0), rapid(731.0, 0.0, 5.0)]);
        let error = machine.check(&wide).unwrap_err().to_string();
        assert!(error.contains("in X"), "{error}");

        // An arc counts its whole circle: its two ends alone span nothing.
        let arc = program(vec![
            rapid(0.0, 0.0, 5.0),
            Step::Move(Move::Arc {
                to: Point3::new(0.0, 0.0, 5.0),
                center: Point3::new(0.0, 366.0, 5.0),
                direction: crate::ArcDirection::Clockwise,
                feed: 100.0,
            }),
        ]);
        let error = machine.check(&arc).unwrap_err().to_string();
        assert!(error.contains("spans 732.000 mm"), "{error}");
    }

    #[test]
    fn the_table_turns_in_its_own_steps() {
        let machine = Machine::from_toml(MAZAK).unwrap();
        let index = |axis: &str, angle: f64| {
            program(vec![
                Step::Event(Event::Home),
                Step::Event(Event::Index {
                    axis: axis.into(),
                    angle,
                }),
            ])
        };
        machine.check(&index("B", 180.0)).unwrap();
        let error = machine.check(&index("B", 90.5)).unwrap_err().to_string();
        assert!(error.contains("whole number"), "{error}");
        let error = machine.check(&index("A", 90.0)).unwrap_err().to_string();
        assert!(error.contains("no A axis"), "{error}");
        // Each side is measured alone: two 500 mm sides fit.
        let two_sides = program(vec![
            rapid(0.0, 0.0, 5.0),
            rapid(500.0, 0.0, 5.0),
            Step::Event(Event::Home),
            Step::Event(Event::Index {
                axis: "B".into(),
                angle: 180.0,
            }),
            Step::Event(Event::WorkOffset { offset: 2 }),
            rapid(-500.0, 0.0, 5.0),
            rapid(0.0, 0.0, 5.0),
        ]);
        machine.check(&two_sides).unwrap();
    }

    #[test]
    fn tapping_needs_rigid_tapping() {
        let text = MAZAK.replace("rigid_tapping = true", "rigid_tapping = false");
        let machine = Machine::from_toml(&text).unwrap();
        let tap = program(vec![
            tool(6),
            spindle(500.0),
            rapid(10.0, 10.0, 5.0),
            Step::Move(Move::Cycle(Cycle {
                kind: CycleKind::Tap { pitch: 1.0 },
                holes: vec![Point3::new(10.0, 10.0, 0.0)],
                clearance: 2.0,
                bottom: -12.0,
                feed: 500.0,
            })),
        ]);
        let error = machine.check(&tap).unwrap_err().to_string();
        assert!(error.contains("cannot tap rigidly"), "{error}");
    }
}
