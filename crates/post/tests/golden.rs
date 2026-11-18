// SPDX-License-Identifier: AGPL-3.0-or-later
//! Golden-file tests: each shipped post must turn the sample program into
//! exactly the G-code in `tests/golden/<post id>.nc`.
//!
//! A difference fails the test. If the change is intended, regenerate with
//! `CNDT_UPDATE_GOLDEN=1 cargo test -p cndt-post` and review every changed
//! line before committing (docs/safety.md, gate 3).

// This whole file is test code; the no-panic rule is for library code.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use cndt_cam_core::{
    ArcDirection, Coolant, Cycle, CycleKind, Event, Move, Operation, Program, SpindleDirection,
    Step,
};
use cndt_geom::Point3;
use cndt_post::{Post, PostError, UNKNOWN_HEIGHT, read_back};

/// A fixed version, so golden files do not change with every release.
const APP_VERSION: &str = "0.0.0-golden";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(id: &str) -> Post {
    let text = std::fs::read_to_string(root().join(format!("posts/{id}.toml"))).unwrap();
    Post::from_toml(&text).unwrap()
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn rapid(x: f64, y: f64, z: f64) -> Step {
    Step::Move(Move::Rapid { to: p(x, y, z) })
}

fn cut(x: f64, y: f64, z: f64, feed: f64) -> Step {
    Step::Move(Move::Linear {
        to: p(x, y, z),
        feed,
    })
}

/// Faces the top of a 40 x 20 mm plate, cuts a 10 mm circle, then drills
/// two holes with a dwell. Covers every kind of step at least once.
fn sample() -> Program {
    let tool = |number, description: &str| {
        Step::Event(Event::ToolChange {
            number,
            description: description.to_owned(),
        })
    };
    let spindle = |rpm| {
        Step::Event(Event::SpindleOn {
            rpm,
            direction: SpindleDirection::Clockwise,
        })
    };
    Program {
        name: String::from("Bracket (top)"),
        number: 1001,
        work_offset: 1,
        operations: vec![
            Operation {
                name: String::from("Face"),
                steps: vec![
                    tool(1, "50 mm face mill"),
                    spindle(6000.0),
                    Step::Event(Event::Coolant {
                        mode: Coolant::Flood,
                    }),
                    rapid(-30.0, 0.0, 5.0),
                    rapid(-30.0, 0.0, 0.0),
                    cut(70.0, 0.0, 0.0, 800.0),
                    cut(70.0, 20.0, 0.0, 800.0),
                    cut(-30.0, 20.0, 0.0, 800.0),
                    // A move to where the tool already is: no output.
                    cut(-30.0, 20.0, 0.0, 800.0),
                    rapid(-30.0, 20.0, 5.0),
                ],
            },
            Operation {
                name: String::from("Circle"),
                steps: vec![
                    tool(2, "6 mm flat end mill"),
                    spindle(12000.0),
                    rapid(25.0, 10.0, 5.0),
                    cut(25.0, 10.0, -1.5, 200.0),
                    Step::Move(Move::Arc {
                        to: p(15.0, 10.0, -1.5),
                        center: p(20.0, 10.0, -1.5),
                        direction: ArcDirection::CounterClockwise,
                        feed: 600.0,
                    }),
                    Step::Move(Move::Arc {
                        to: p(25.0, 10.0, -1.5),
                        center: p(20.0, 10.0, -1.5),
                        direction: ArcDirection::CounterClockwise,
                        feed: 600.0,
                    }),
                    rapid(25.0, 10.0, 5.0),
                    Step::Event(Event::ProgramStop { optional: true }),
                ],
            },
            Operation {
                name: String::from("Drill"),
                steps: vec![
                    tool(3, "5 mm drill"),
                    spindle(2500.0),
                    Step::Event(Event::Comment {
                        text: String::from("Check: clamps clear of holes"),
                    }),
                    rapid(5.0, 5.0, 5.0),
                    cut(5.0, 5.0, -8.0, 120.0),
                    Step::Move(Move::Dwell { seconds: 0.5 }),
                    rapid(5.0, 5.0, 5.0),
                    rapid(35.0, 5.0, 5.0),
                    cut(35.0, 5.0, -8.0, 120.0),
                    Step::Move(Move::Dwell { seconds: 0.5 }),
                    rapid(35.0, 5.0, 5.0),
                    Step::Event(Event::Coolant { mode: Coolant::Off }),
                    Step::Event(Event::SpindleOff),
                ],
            },
        ],
    }
}

/// Drills two holes with each kind of cycle: drill, peck, tap (when
/// `tap` is set) and ream.
fn cycles(tap: bool) -> Program {
    let holes = vec![p(10.0, 10.0, 0.0), p(30.0, 10.0, 0.0)];
    let op =
        |name: &str, tool: u32, what: &str, rpm: f64, kind: CycleKind, bottom: f64, feed: f64| {
            Operation {
                name: name.to_owned(),
                steps: vec![
                    Step::Event(Event::ToolChange {
                        number: tool,
                        description: what.to_owned(),
                    }),
                    Step::Event(Event::SpindleOn {
                        rpm,
                        direction: SpindleDirection::Clockwise,
                    }),
                    Step::Event(Event::Coolant {
                        mode: Coolant::Flood,
                    }),
                    rapid(10.0, 10.0, 10.0),
                    Step::Move(Move::Cycle(Cycle {
                        kind,
                        holes: holes.clone(),
                        clearance: 2.0,
                        bottom,
                        feed,
                    })),
                    Step::Event(Event::Coolant { mode: Coolant::Off }),
                ],
            }
        };
    let mut operations = vec![
        op(
            "Drill",
            2,
            "5 mm drill",
            2500.0,
            CycleKind::Drill,
            -6.0,
            250.0,
        ),
        op(
            "Peck",
            2,
            "5 mm drill",
            2500.0,
            CycleKind::Peck { step: 5.0 },
            -12.0,
            250.0,
        ),
    ];
    if tap {
        operations.push(op(
            "Tap",
            6,
            "M6 x 1.0 tap",
            600.0,
            CycleKind::Tap { pitch: 1.0 },
            -12.0,
            600.0,
        ));
    }
    operations.push(op(
        "Ream",
        7,
        "8 mm H7 reamer",
        400.0,
        CycleKind::Ream,
        -12.0,
        120.0,
    ));
    Program {
        name: String::from("Cycles"),
        number: 1002,
        work_offset: 1,
        operations,
    }
}

fn check_golden(id: &str) {
    compare(id, id, &sample());
}

fn compare(id: &str, file: &str, program: &Program) {
    let output = load(id).run(program, APP_VERSION).unwrap();
    let path = root().join(format!("crates/post/tests/golden/{file}.nc"));
    if std::env::var_os("CNDT_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &output).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if output != expected {
        panic!(
            "{id}: output differs from {}.\n\
             If the change is intended, run `CNDT_UPDATE_GOLDEN=1 cargo test -p cndt-post`\n\
             and review every changed line.\n\n--- got ---\n{output}",
            path.display()
        );
    }
}

#[test]
fn fanuc_mill_matches_golden() {
    check_golden("fanuc-mill");
}

#[test]
fn grbl_matches_golden() {
    check_golden("grbl");
}

#[test]
fn haas_mill_matches_golden() {
    check_golden("haas-mill");
}

#[test]
fn mazak_smoothg_matches_golden() {
    check_golden("mazak-smoothg");
}

#[test]
fn linuxcnc_mill_matches_golden() {
    check_golden("linuxcnc-mill");
}

#[test]
fn canned_cycles_match_golden() {
    compare("fanuc-mill", "fanuc-mill-cycles", &cycles(true));
    compare("haas-mill", "haas-mill-cycles", &cycles(true));
    // Mazak taps synchronously with G84.2.
    compare("mazak-smoothg", "mazak-smoothg-cycles", &cycles(true));
    // LinuxCNC taps with G33.1, one hole at a time.
    compare("linuxcnc-mill", "linuxcnc-mill-cycles", &cycles(true));
    // GRBL has no canned cycles: they are written out as moves.
    compare("grbl", "grbl-cycles", &cycles(false));
}

/// Two sides of a part: the first face, then either a turn of the B table
/// or a stop for the operator to turn the part, then the second face on
/// its own work offset with the same tool.
fn two_sides(indexed: bool) -> Program {
    let side = |name: &str| Operation {
        name: name.to_owned(),
        steps: vec![
            Step::Event(Event::SpindleOn {
                rpm: 6000.0,
                direction: SpindleDirection::Clockwise,
            }),
            rapid(-30.0, 0.0, 5.0),
            rapid(-30.0, 0.0, 0.0),
            cut(70.0, 0.0, 0.0, 800.0),
            rapid(70.0, 0.0, 5.0),
            Step::Event(Event::SpindleOff),
        ],
    };
    let mut first = side("Face side 1");
    first.steps.insert(
        0,
        Step::Event(Event::ToolChange {
            number: 1,
            description: String::from("50 mm face mill"),
        }),
    );
    let mut turn = vec![Step::Event(Event::Home)];
    if indexed {
        turn.push(Step::Event(Event::Index {
            axis: String::from("B"),
            angle: 180.0,
        }));
    } else {
        turn.push(Step::Event(Event::Comment {
            text: String::from("Turn the part over and clamp it"),
        }));
        turn.push(Step::Event(Event::ProgramStop { optional: false }));
    }
    turn.push(Step::Event(Event::WorkOffset { offset: 2 }));
    Program {
        name: String::from("Plate both sides"),
        number: 1002,
        work_offset: 1,
        operations: vec![
            first,
            Operation {
                name: String::from(if indexed { "B180" } else { "Turn the part" }),
                steps: turn,
            },
            side("Face side 2"),
        ],
    }
}

#[test]
fn two_sides_match_golden() {
    // The B table turns, and the length offset comes back after going home.
    compare("mazak-smoothg", "mazak-smoothg-indexed", &two_sides(true));
    // The operator turns the part.
    compare("fanuc-mill", "fanuc-mill-turned", &two_sides(false));
}

#[test]
fn posts_refuse_what_their_machine_cannot_do() {
    let error = load("fanuc-mill")
        .run(&two_sides(true), APP_VERSION)
        .unwrap_err();
    assert!(
        error.to_string().contains("cannot turn a rotary axis (B)"),
        "{error}"
    );
    // GRBL has no safe home position to go to.
    let error = load("grbl")
        .run(&two_sides(false), APP_VERSION)
        .unwrap_err();
    assert!(error.to_string().contains("no home lines"), "{error}");
}

#[test]
fn a_post_without_rigid_tapping_refuses_to_tap() {
    let error = load("grbl").run(&cycles(true), APP_VERSION).unwrap_err();
    assert!(matches!(error, PostError::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("rigid tapping"), "{error}");
}

#[test]
fn every_shipped_post_loads() {
    let mut count = 0;
    for entry in std::fs::read_dir(root().join("posts")).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            let text = std::fs::read_to_string(&path).unwrap();
            Post::from_toml(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            count += 1;
        }
    }
    assert!(count >= 5);
}

#[test]
fn an_override_is_recorded_in_the_header() {
    let notes = [
        String::from("VERIFICATION OVERRIDDEN BY THE USER:"),
        String::from("Setup 1: a rapid (move) runs 1.50 mm into the stock"),
    ];
    let output = load("fanuc-mill")
        .run_noted(&sample(), APP_VERSION, &notes)
        .unwrap();
    let header: Vec<&str> = output.lines().skip(2).take(6).collect();
    assert_eq!(
        header,
        [
            "(CNDT 0.0.0-GOLDEN)",
            "(POST FANUC-MILL 0.1.0 UNVERIFIED)",
            "(UNVERIFIED POST - PROVE OUT CAREFULLY BEFORE RUNNING)",
            "(PREVIEW BUILD - NOT FOR PRODUCTION)",
            "(VERIFICATION OVERRIDDEN BY THE USER:)",
            "(SETUP 1: A RAPID MOVE RUNS 1.50 MM INTO THE STOCK)",
        ]
    );
    // Everything after the header is the same as without notes.
    let plain = load("fanuc-mill").run(&sample(), APP_VERSION).unwrap();
    assert_eq!(
        output.lines().skip(8).collect::<Vec<_>>(),
        plain.lines().skip(6).collect::<Vec<_>>()
    );
}

#[test]
fn output_is_deterministic() {
    let post = load("fanuc-mill");
    assert_eq!(
        post.run(&sample(), APP_VERSION).unwrap(),
        post.run(&sample(), APP_VERSION).unwrap()
    );
}

#[test]
fn every_program_carries_the_safety_header() {
    for id in [
        "fanuc-mill",
        "grbl",
        "haas-mill",
        "linuxcnc-mill",
        "mazak-smoothg",
    ] {
        let output = load(id).run(&sample(), "0.1.0").unwrap();
        let upper = output.to_ascii_uppercase();
        assert!(upper.contains("CNDT 0.1.0"), "{id}");
        assert!(
            upper.contains(&format!(
                "POST {} 0.1.0 UNVERIFIED",
                id.to_ascii_uppercase()
            )),
            "{id}"
        );
        assert!(upper.contains("PROVE OUT CAREFULLY"), "{id}");
        assert!(upper.contains("NOT FOR PRODUCTION"), "{id}");
    }
}

#[test]
fn an_unsafe_program_produces_no_gcode() {
    let mut program = sample();
    // Cutting with the spindle off.
    program.operations[2]
        .steps
        .insert(3, Step::Event(Event::SpindleOff));
    let error = load("fanuc-mill").run(&program, APP_VERSION).unwrap_err();
    assert!(matches!(error, PostError::Toolpath(_)), "{error:?}");
    assert!(error.to_string().contains("spindle off"), "{error}");
}

#[test]
fn bad_post_definitions_fail_on_load() {
    let good = std::fs::read_to_string(root().join("posts/fanuc-mill.toml")).unwrap();
    let cases = [
        (good.replace("\"{header}\",\n", ""), "{header}"),
        // {pitch} is only there when each hole is tapped on its own.
        (good.replace("G84 X{x}", "G84 K{pitch} X{x}"), "pitch"),
        (good.replace("M3 S{rpm}", "M3 S{rmp}"), "{rmp}"),
        (good.replace("T{tool} M6", "T{tool M6"), "unclosed"),
        (
            good.replace("verified = false", "verified = false\nverifed = true"),
            "unknown field",
        ),
        (good.replace("decimals = 3", "decimals = 9"), "at most 6"),
    ];
    for (text, expected) in cases {
        let error = Post::from_toml(&text).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in: {error}"
        );
    }
}

#[test]
fn cutting_straight_after_a_tool_change_is_refused_when_compensation_is_used() {
    let mut program = sample();
    // Remove the approach rapid so the first move after the tool change cuts.
    program.operations[1].steps.remove(2);
    let error = load("fanuc-mill").run(&program, APP_VERSION).unwrap_err();
    assert!(matches!(error, PostError::Unsupported(_)), "{error:?}");
    assert!(error.to_string().contains("must be a rapid"), "{error}");
    // GRBL has no tool length compensation, so the same program posts.
    assert!(load("grbl").run(&program, APP_VERSION).is_ok());
}

/// What a program does, in a form the same for any way of writing it: the
/// rapids to known points, every feed and arc, each hole of each cycle with
/// the rapids around it spelled out, and the tool changes, table turns and
/// work offsets, rounded to the posts' three decimals. A move to where the
/// tool already is does nothing, and posts leave it out, so it is skipped.
fn signature(program: &Program) -> Vec<String> {
    let r = |v: f64| format!("{:.3}", (v * 1000.0).round() / 1000.0 + 0.0);
    let pt = |p: &Point3| format!("{} {} {}", r(p.x), r(p.y), r(p.z));
    let mut out = Vec::new();
    let mut at: Option<String> = None;
    let mut z = UNKNOWN_HEIGHT;
    let rapid = |out: &mut Vec<String>, at: &mut Option<String>, to: Point3| {
        let here = pt(&to);
        if at.as_deref() != Some(here.as_str()) && to.z < UNKNOWN_HEIGHT - 1.0 {
            out.push(format!("rapid {here}"));
        }
        *at = Some(here);
    };
    for step in program.operations.iter().flat_map(|o| &o.steps) {
        match step {
            Step::Move(Move::Rapid { to }) => {
                rapid(&mut out, &mut at, *to);
                z = to.z;
            }
            Step::Move(Move::Dwell { .. }) => {}
            Step::Move(Move::Linear { to, feed }) => {
                let here = pt(to);
                if at.as_deref() != Some(here.as_str()) {
                    out.push(format!("feed {here} F{}", r(*feed)));
                }
                at = Some(here);
                z = to.z;
            }
            Step::Move(Move::Arc {
                to,
                center,
                direction,
                feed,
            }) => {
                out.push(format!(
                    "arc {direction:?} {} c{} {} F{}",
                    pt(to),
                    r(center.x),
                    r(center.y),
                    r(*feed)
                ));
                at = Some(pt(to));
                z = to.z;
            }
            Step::Move(Move::Cycle(cycle)) => {
                let kind = match cycle.kind {
                    CycleKind::Drill => String::from("drill"),
                    CycleKind::Peck { step } => format!("peck {}", r(step)),
                    CycleKind::Tap { pitch } => format!("tap {}", r(pitch)),
                    CycleKind::Ream => String::from("ream"),
                };
                for hole in &cycle.holes {
                    rapid(&mut out, &mut at, p(hole.x, hole.y, z));
                    rapid(&mut out, &mut at, p(hole.x, hole.y, cycle.clearance));
                    out.push(format!(
                        "{kind} {} {} R{} Z{} F{}",
                        r(hole.x),
                        r(hole.y),
                        r(cycle.clearance),
                        r(cycle.bottom),
                        r(cycle.feed)
                    ));
                    rapid(&mut out, &mut at, p(hole.x, hole.y, z));
                }
            }
            Step::Event(Event::ToolChange { number, .. }) => out.push(format!("T{number}")),
            Step::Event(Event::Index { axis, angle }) => out.push(format!("{axis}{}", r(*angle))),
            Step::Event(Event::WorkOffset { offset }) => {
                out.push(format!("G{}", 53 + offset));
                at = None;
                z = UNKNOWN_HEIGHT;
            }
            Step::Event(Event::Home) => {
                at = None;
                z = UNKNOWN_HEIGHT;
            }
            Step::Event(_) => {}
        }
    }
    out
}

#[test]
fn posted_programs_read_back_to_what_was_posted() {
    let programs = [
        ("sample", sample()),
        ("cycles", cycles(true)),
        ("indexed", two_sides(true)),
        ("turned", two_sides(false)),
    ];
    // GRBL writes no tool numbers, so its programs are refused, not misread.
    let grbl = load("grbl").run(&sample(), APP_VERSION).unwrap();
    let error = read_back(&grbl).unwrap_err();
    assert!(
        error.to_string().contains("before any tool number"),
        "{error}"
    );
    for id in ["fanuc-mill", "haas-mill", "mazak-smoothg", "linuxcnc-mill"] {
        let post = load(id);
        for (name, program) in &programs {
            let Ok(gcode) = post.run(program, APP_VERSION) else {
                continue; // a program this post refuses (a table turn, say)
            };
            let back = read_back(&gcode).unwrap_or_else(|e| panic!("{id} {name}: {e}\n{gcode}"));
            assert_eq!(back.work_offset, program.work_offset, "{id} {name}");
            let (want, got) = (signature(program), signature(&back));
            assert_eq!(
                want, got,
                "{id} {name}: the G-code does not read back to the program\n{gcode}"
            );
        }
    }
}

#[test]
fn what_cannot_be_read_back_is_refused_with_the_line() {
    for (gcode, reason) in [
        ("G0 X1 Y1 Z1\nG91 G1 X5 F100\n", "incremental"),
        ("T1 M6\nG0 X0 Y0 Z5\nG2 X10 Y0 R5 F100\n", "radius"),
        ("G0 X0 Y0 Z5\nG99 G81 X0 Y0 Z-5 R2 F100\n", "G99"),
        ("T1 M6\nG0 X0 Y0 Z5\nG1 X5\n", "no feed rate"),
        ("G7 X0\n", "unknown code G7"),
        ("G0 X0 Y0 Z5\nW3\n", "unknown word W"),
        (
            "G0 X0 Y0 Z5\nM3 S1000\nG1 Z-1 F100\n",
            "before any tool number",
        ),
    ] {
        let error = read_back(gcode).unwrap_err();
        assert!(error.to_string().contains(reason), "{gcode}: {error}");
        assert_eq!(error.line, gcode.lines().count(), "{gcode}");
    }
}

const SHOP_HAAS: &str = include_str!("posts/shop-haas.toml");

#[test]
fn an_extending_post_keeps_the_base_and_changes_what_it_sets() {
    let post = Post::from_toml(SHOP_HAAS).unwrap();
    let base = load("haas-mill");
    let (def, base_def) = (post.definition(), base.definition());
    assert_eq!(def.post.id, "shop-haas");
    assert_eq!(def.format.decimals, 4);
    assert_eq!(def.program.end.len(), 4);
    assert_eq!(def.format.line_numbers.unwrap().start, 100);
    // Untouched settings come from the base.
    assert_eq!(def.codes.tool_change, base_def.codes.tool_change);
    assert_eq!(def.program.start, base_def.program.start);
    assert_eq!(def.format.feed_decimals, base_def.format.feed_decimals);
    assert_ne!(def.program.end, base_def.program.end);
}

#[test]
fn an_extending_post_matches_its_golden_file() {
    let output = Post::from_toml(SHOP_HAAS)
        .unwrap()
        .run(&sample(), APP_VERSION)
        .unwrap();
    let path = root().join("crates/post/tests/golden/shop-haas.nc");
    if std::env::var_os("CNDT_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &output).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(output, expected, "output differs from {}", path.display());
    assert!(
        output.contains("(POST SHOP-HAAS 1.0.0 UNVERIFIED)"),
        "{output}"
    );
}

#[test]
fn an_extending_post_is_never_verified() {
    let verified = SHOP_HAAS.replace(
        "version = \"1.0.0\"",
        "version = \"1.0.0\"\nverified = true",
    );
    let error = Post::from_toml(&verified).unwrap_err().to_string();
    assert!(error.contains("cannot be verified"), "{error}");
    assert!(
        !Post::from_toml(SHOP_HAAS)
            .unwrap()
            .definition()
            .post
            .verified
    );
}

#[test]
fn a_bad_extends_is_refused_with_a_reason() {
    let cases = [
        (
            SHOP_HAAS.replace("\"haas-mill\"", "\"nope\""),
            "not a shipped post",
        ),
        (SHOP_HAAS.replace("\"haas-mill\"", "3"), "must be a post id"),
        (
            SHOP_HAAS.replace("id = \"shop-haas\"", "id = \"haas-mill\""),
            "must differ",
        ),
        (
            SHOP_HAAS.replace("name = \"Shop Haas\"\n", ""),
            "must set post.name",
        ),
        (
            SHOP_HAAS.replace("decimals = 4", "decimals = 4\ndecimls = 5"),
            "unknown field",
        ),
        (
            SHOP_HAAS.replace("decimals = 4", "decimals = 9"),
            "at most 6",
        ),
        (SHOP_HAAS.replace("\"M30\",\n]", "\"M30 {x}\",\n]"), "{x}"),
    ];
    for (text, expected) in cases {
        let error = Post::from_toml(&text).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in: {error}"
        );
    }
}

/// The shipped GRBL post's text with one line replaced, for refusal tests.
fn grbl_with(from: &str, to: &str) -> String {
    let text = std::fs::read_to_string(root().join("posts/grbl.toml")).unwrap();
    assert!(text.contains(from), "grbl.toml no longer contains {from:?}");
    text.replacen(from, to, 1)
}

#[test]
fn a_post_definition_that_could_post_unsafely_is_refused_with_a_reason() {
    let refused = |text: String, wanted: &str| {
        let error = Post::from_toml(&text).unwrap_err();
        assert!(matches!(error, PostError::InvalidDefinition(_)), "{error}");
        assert!(error.to_string().contains(wanted), "{wanted}: {error}");
    };
    // The safety header cannot be left out.
    refused(grbl_with("    \"{header}\",\n", ""), "{header}");
    // More decimals than the formatter supports.
    refused(grbl_with("decimals = 3", "decimals = 9"), "decimals");
    // A placeholder the post does not define.
    refused(
        grbl_with("\"G21 G17 G90 G94\"", "\"G21 {nonsense}\""),
        "nonsense",
    );
    // A misspelt field does not pass silently.
    refused(grbl_with("decimals = 3", "decimalz = 3"), "decimalz");
    // Not TOML at all.
    refused(String::from("this is [not toml"), "invalid post definition");
}

#[test]
fn a_program_that_is_not_safe_is_not_posted() {
    let unsafe_program = Program {
        name: String::from("bad"),
        number: 1,
        work_offset: 9,
        operations: vec![Operation {
            name: String::from("op"),
            steps: vec![rapid(0.0, 0.0, 5.0)],
        }],
    };
    for id in [
        "fanuc-mill",
        "grbl",
        "haas-mill",
        "linuxcnc-mill",
        "mazak-smoothg",
    ] {
        let error = load(id).run(&unsafe_program, APP_VERSION).unwrap_err();
        assert!(matches!(error, PostError::Toolpath(_)), "{id}: {error}");
        assert!(error.to_string().contains("work offset"), "{id}: {error}");
    }
}
