// SPDX-License-Identifier: AGPL-3.0-or-later
//! Comparing the kind and feed rate of the moves in two toolpaths (decision
//! 0016, docs/safety.md "Known gap"): the planned one and the posted G-code
//! read back. Material removed is compared by the simulation; this catches
//! what removes no material, a wrong `F` or a cutting move posted as a rapid.

use cndt_cam_core::{Coolant, CycleKind, Event, Move, Program, SpindleDirection, Step};

/// The posts write feed rates with at most whole mm/min, so a difference up
/// to half of that is rounding, not an error.
const FEED_TOLERANCE: f64 = 0.5 + 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Rapid,
    Cut(f64),
    Cycle(u8, Option<f64>, usize),
}

impl Item {
    fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Rapid, Self::Rapid) => true,
            (Self::Cut(a), Self::Cut(b)) => (a - b).abs() <= FEED_TOLERANCE,
            (Self::Cycle(k, f, n), Self::Cycle(k2, f2, n2)) => {
                k == k2
                    && n == n2
                    && match (f, f2) {
                        (Some(a), Some(b)) => (a - b).abs() <= FEED_TOLERANCE,
                        _ => true,
                    }
            }
            _ => false,
        }
    }

    fn describe(self) -> String {
        match self {
            Self::Rapid => String::from("a rapid move"),
            Self::Cut(f) => format!("a cutting move at F{f:.0}"),
            Self::Cycle(kind, feed, holes) => {
                let kind = ["drilling", "peck drilling", "tapping", "reaming"]
                    .get(usize::from(kind))
                    .copied()
                    .unwrap_or("drilling");
                let feed = feed.map_or(String::new(), |f| format!(" at F{f:.0}"));
                format!(
                    "a {kind} cycle{feed} over {holes} hole{}",
                    if holes == 1 { "" } else { "s" }
                )
            }
        }
    }
}

/// The moves of a program as runs: consecutive moves of the same kind and
/// feed are one run, because a post may split an arc or join two rapids.
fn runs(program: &Program) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for step in program.operations.iter().flat_map(|o| &o.steps) {
        let Step::Move(m) = step else { continue };
        let item = match m {
            Move::Rapid { .. } => Item::Rapid,
            Move::Linear { feed, .. } | Move::Arc { feed, .. } => Item::Cut(*feed),
            Move::Cycle(c) => {
                let (kind, feed) = match c.kind {
                    CycleKind::Drill => (0, Some(c.feed)),
                    CycleKind::Peck { .. } => (1, Some(c.feed)),
                    CycleKind::Tap { .. } => (2, None),
                    CycleKind::Ream => (3, Some(c.feed)),
                };
                Item::Cycle(kind, feed, c.holes.len())
            }
            _ => continue,
        };
        // A cycle returns by rapid, and a post that taps hole by hole
        // positions by rapid too: a rapid right after a cycle is part of it.
        if matches!((out.last(), item), (Some(Item::Cycle(..)), Item::Rapid)) {
            continue;
        }
        match (out.last_mut(), item) {
            // Cycles of one kind and feed in a row are one run over all
            // their holes: a post may tap each hole by itself.
            (Some(Item::Cycle(k, f, n)), Item::Cycle(k2, f2, n2))
                if *k == k2 && Item::Cycle(*k, *f, 0).same(Item::Cycle(k2, f2, 0)) =>
            {
                *n += n2;
            }
            (Some(last), item) if !matches!(item, Item::Cycle(..)) && last.same(item) => {}
            _ => out.push(item),
        }
    }
    out
}

/// `None` when the posted program's moves are, in order, the planned ones'
/// kinds and feed rates; otherwise what differs first.
#[must_use]
pub(crate) fn compare(planned: &Program, back: &Program) -> Option<String> {
    let (a, b) = (runs(planned), runs(back));
    for (i, (p, q)) in a.iter().zip(&b).enumerate() {
        if !p.same(*q) {
            return Some(format!(
                "the posted G-code moves differently from the plan: motion {} should be {} \
                 but is {}. The post may be wrong; do not run it.",
                i + 1,
                p.describe(),
                q.describe()
            ));
        }
    }
    let n = a.len().min(b.len());
    match (a.get(n), b.get(n)) {
        (Some(p), None) => Some(format!(
            "the plan has {} (motion {}) that the posted G-code lacks, or posts as part of \
             another move. The post may be wrong; do not run it.",
            p.describe(),
            n + 1
        )),
        (None, Some(q)) => Some(format!(
            "the posted G-code has {} (motion {}) that the plan does not. \
             The post may be wrong; do not run it.",
            q.describe(),
            n + 1
        )),
        _ => None,
    }
}

/// The machine state a cutting move runs in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct State {
    tool: Option<u32>,
    /// Speed and direction; `None` while the spindle is stopped.
    spindle: Option<(f64, SpindleDirection)>,
    coolant: Coolant,
}

impl State {
    fn same(self, other: Self) -> bool {
        self.tool == other.tool
            && self.coolant == other.coolant
            && match (self.spindle, other.spindle) {
                (None, None) => true,
                (Some((a, d)), Some((b, e))) => d == e && (a - b).abs() <= FEED_TOLERANCE,
                _ => false,
            }
    }

    fn describe(self) -> String {
        let tool = self
            .tool
            .map_or(String::from("no tool"), |t| format!("T{t}"));
        let spindle = match self.spindle {
            None => String::from("the spindle stopped"),
            Some((rpm, SpindleDirection::Clockwise)) => format!("S{rpm:.0} clockwise"),
            Some((rpm, SpindleDirection::CounterClockwise)) => {
                format!("S{rpm:.0} counter-clockwise")
            }
        };
        let coolant = match self.coolant {
            Coolant::Off => "no coolant",
            Coolant::Flood => "flood coolant",
            Coolant::Mist => "mist coolant",
        };
        format!("{tool}, {spindle}, {coolant}")
    }
}

/// The distinct states the cutting moves of a program run in, in order.
fn states(program: &Program) -> Vec<State> {
    let mut now = State {
        tool: None,
        spindle: None,
        coolant: Coolant::Off,
    };
    let mut out: Vec<State> = Vec::new();
    for step in program.operations.iter().flat_map(|o| &o.steps) {
        match step {
            Step::Event(Event::ToolChange { number, .. }) => now.tool = Some(*number),
            Step::Event(Event::SpindleOn { rpm, direction }) => {
                now.spindle = Some((*rpm, *direction));
            }
            Step::Event(Event::SpindleOff) => now.spindle = None,
            Step::Event(Event::Coolant { mode }) => now.coolant = *mode,
            Step::Move(Move::Linear { .. } | Move::Arc { .. } | Move::Cycle(_))
                if !out.last().is_some_and(|last| last.same(now)) =>
            {
                out.push(now);
            }
            _ => {}
        }
    }
    out
}

/// `None` when the cutting moves of the posted program run with the
/// planned tool, spindle speed and direction and coolant, in order;
/// otherwise what differs first.
#[must_use]
pub(crate) fn compare_state(planned: &Program, back: &Program) -> Option<String> {
    let (a, b) = (states(planned), states(back));
    let hint = "The post may be wrong; do not run it.";
    for (i, (p, q)) in a.iter().zip(&b).enumerate() {
        if !p.same(*q) {
            return Some(format!(
                "the posted G-code cuts in a different machine state from the plan: cutting \
                 stretch {} should run with {} but runs with {}. {hint}",
                i + 1,
                p.describe(),
                q.describe()
            ));
        }
    }
    if a.len() == b.len() {
        None
    } else {
        Some(format!(
            "the plan cuts in {} machine states and the posted G-code in {}. {hint}",
            a.len(),
            b.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cndt_cam_core::Operation;
    use cndt_geom::Point3;

    fn program(moves: Vec<Move>) -> Program {
        Program {
            name: String::new(),
            number: 1,
            work_offset: 1,
            operations: vec![Operation {
                name: String::new(),
                steps: moves.into_iter().map(Step::Move).collect(),
            }],
        }
    }

    fn rapid(x: f64) -> Move {
        Move::Rapid {
            to: Point3::new(x, 0.0, 5.0),
        }
    }

    fn cut(x: f64, feed: f64) -> Move {
        Move::Linear {
            to: Point3::new(x, 0.0, -1.0),
            feed,
        }
    }

    #[test]
    fn identical_motion_agrees() {
        let a = program(vec![
            rapid(0.0),
            cut(1.0, 500.0),
            cut(2.0, 500.0),
            rapid(3.0),
        ]);
        // Split or joined moves of one kind are the same run.
        let b = program(vec![rapid(0.0), cut(1.5, 500.2), rapid(3.0)]);
        assert_eq!(compare(&a, &b), None);
    }

    #[test]
    fn a_wrong_feed_is_found() {
        let a = program(vec![rapid(0.0), cut(1.0, 500.0), rapid(3.0)]);
        let b = program(vec![rapid(0.0), cut(1.0, 5000.0), rapid(3.0)]);
        let found = compare(&a, &b).expect("differs");
        assert!(found.contains("F500") && found.contains("F5000"), "{found}");
    }

    #[test]
    fn a_cut_posted_as_a_rapid_is_found() {
        let a = program(vec![rapid(0.0), cut(1.0, 500.0), rapid(3.0)]);
        let b = program(vec![rapid(0.0), rapid(1.0), rapid(3.0)]);
        let found = compare(&a, &b).expect("differs");
        assert!(found.contains("a cutting move at F500"), "{found}");
        // A rapid posted as a cut is found too.
        assert!(compare(&b, &a).is_some());
    }

    fn with_state(tool: u32, rpm: f64, coolant: Coolant) -> Program {
        Program {
            name: String::new(),
            number: 1,
            work_offset: 1,
            operations: vec![Operation {
                name: String::new(),
                steps: vec![
                    Step::Event(Event::ToolChange {
                        number: tool,
                        description: String::new(),
                    }),
                    Step::Event(Event::SpindleOn {
                        rpm,
                        direction: SpindleDirection::Clockwise,
                    }),
                    Step::Event(Event::Coolant { mode: coolant }),
                    Step::Move(rapid(0.0)),
                    Step::Move(cut(1.0, 500.0)),
                ],
            }],
        }
    }

    #[test]
    fn same_state_agrees_and_rounding_is_tolerated() {
        let a = with_state(3, 8000.0, Coolant::Flood);
        assert_eq!(
            compare_state(&a, &with_state(3, 8000.4, Coolant::Flood)),
            None
        );
    }

    #[test]
    fn a_wrong_spindle_speed_is_found() {
        let a = with_state(3, 8000.0, Coolant::Off);
        let found = compare_state(&a, &with_state(3, 800.0, Coolant::Off)).expect("differs");
        assert!(
            found.contains("S8000") && found.contains("S800 "),
            "{found}"
        );
    }

    #[test]
    fn a_missing_coolant_is_found() {
        let a = with_state(3, 8000.0, Coolant::Flood);
        let found = compare_state(&a, &with_state(3, 8000.0, Coolant::Off)).expect("differs");
        assert!(
            found.contains("flood coolant") && found.contains("no coolant"),
            "{found}"
        );
    }

    #[test]
    fn a_wrong_tool_is_found() {
        let a = with_state(3, 8000.0, Coolant::Off);
        let found = compare_state(&a, &with_state(4, 8000.0, Coolant::Off)).expect("differs");
        assert!(found.contains("T3") && found.contains("T4"), "{found}");
    }

    #[test]
    fn a_wrong_direction_or_a_stopped_spindle_is_found() {
        let a = with_state(3, 8000.0, Coolant::Off);
        let mut b = a.clone();
        b.operations[0].steps[1] = Step::Event(Event::SpindleOn {
            rpm: 8000.0,
            direction: SpindleDirection::CounterClockwise,
        });
        assert!(compare_state(&a, &b).is_some());
        b.operations[0].steps[1] = Step::Event(Event::SpindleOff);
        assert!(compare_state(&a, &b).is_some());
    }

    #[test]
    fn a_missing_move_is_found() {
        let a = program(vec![rapid(0.0), cut(1.0, 500.0), rapid(3.0)]);
        let b = program(vec![rapid(0.0), cut(1.0, 500.0)]);
        assert!(compare(&a, &b).is_some());
    }
}
