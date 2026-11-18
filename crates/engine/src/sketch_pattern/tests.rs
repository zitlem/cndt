// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::Session;
use crate::sketch_commands::{CreateSketch, EditSketch, PointRef, SketchEdit};
use cndt_model::{Document, Plane, SketchId};

type Segment = ((f64, f64), (f64, f64));

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn pt(sketch: &mut Sketch, x: f64, y: f64) -> EntityId {
    sketch.add_point(Point2::new(x, y)).unwrap()
}

fn line(sketch: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> EntityId {
    let (p, q) = (pt(sketch, a.0, a.1), pt(sketch, b.0, b.1));
    sketch.add_line(p, q).unwrap()
}

/// A vertical mirror line at x = 0.
fn y_axis(sketch: &mut Sketch) -> EntityId {
    line(sketch, (0.0, -50.0), (0.0, 50.0))
}

fn ends(sketch: &Sketch, id: EntityId) -> Segment {
    let Some(Geometry::Line { start, end }) = sketch.entity(id).map(|e| e.geometry.clone()) else {
        panic!("not a line");
    };
    let (a, b) = (sketch.point(start).unwrap(), sketch.point(end).unwrap());
    ((a.x, a.y), (b.x, b.y))
}

fn lines_after(sketch: &Sketch, after: u32) -> Vec<Segment> {
    sketch
        .entities()
        .filter(|(id, e)| id.0 > after && matches!(e.geometry, Geometry::Line { .. }))
        .map(|(id, _)| ends(sketch, id))
        .collect()
}

fn last_id(sketch: &Sketch) -> u32 {
    sketch.entities().map(|(id, _)| id.0).max().unwrap_or(0)
}

fn count_of(sketch: &Sketch, kind: &str) -> usize {
    sketch
        .constraints()
        .filter(|(_, c)| c.kind() == kind)
        .count()
}

fn point_count(sketch: &Sketch) -> usize {
    sketch
        .entities()
        .filter(|(_, e)| matches!(e.geometry, Geometry::Point { .. }))
        .count()
}

#[test]
fn a_mirrored_line_lands_on_the_other_side_with_symmetric_points() {
    let mut sketch = Sketch::new();
    let axis = y_axis(&mut sketch);
    let original = line(&mut sketch, (10.0, 0.0), (20.0, 5.0));
    let before = last_id(&sketch);
    mirror(&mut sketch, &[original], axis).unwrap();
    let copies = lines_after(&sketch, before);
    assert_eq!(copies.len(), 1);
    let ((x0, y0), (x1, y1)) = copies[0];
    assert!(near(x0, -10.0) && near(y0, 0.0) && near(x1, -20.0) && near(y1, 5.0));
    assert_eq!(count_of(&sketch, "symmetric"), 2);
}

#[test]
fn a_slanted_mirror_line_reflects_properly() {
    let mut sketch = Sketch::new();
    // The line y = x.
    let axis = line(&mut sketch, (0.0, 0.0), (10.0, 10.0));
    let original = line(&mut sketch, (4.0, 0.0), (8.0, 0.0));
    let before = last_id(&sketch);
    mirror(&mut sketch, &[original], axis).unwrap();
    let ((x0, y0), (x1, y1)) = lines_after(&sketch, before)[0];
    assert!(near(x0, 0.0) && near(y0, 4.0) && near(x1, 0.0) && near(y1, 8.0));
}

#[test]
fn a_mirrored_chain_stays_joined_and_points_on_the_axis_are_shared() {
    let mut sketch = Sketch::new();
    let axis = y_axis(&mut sketch);
    let a = pt(&mut sketch, 0.0, 0.0);
    let b = pt(&mut sketch, 10.0, 0.0);
    let c = pt(&mut sketch, 10.0, 10.0);
    let first = sketch.add_line(a, b).unwrap();
    let second = sketch.add_line(b, c).unwrap();
    let before = last_id(&sketch);
    let points_before = point_count(&sketch);
    mirror(&mut sketch, &[first, second], axis).unwrap();
    // b and c are copied; a is on the axis and shared.
    assert_eq!(point_count(&sketch) - points_before, 2);
    assert_eq!(count_of(&sketch, "symmetric"), 2);
    let copies = lines_after(&sketch, before);
    assert_eq!(copies.len(), 2);
    // The two copies meet at (-10, 0).
    let at_corner = |p: (f64, f64)| near(p.0, -10.0) && near(p.1, 0.0);
    let meeting = copies
        .iter()
        .filter(|l| at_corner(l.0) || at_corner(l.1))
        .count();
    assert_eq!(meeting, 2);
}

#[test]
fn a_mirrored_circle_has_the_same_radius_and_symmetric_centre() {
    let mut sketch = Sketch::new();
    let axis = y_axis(&mut sketch);
    let c = pt(&mut sketch, 15.0, 3.0);
    let circle = sketch.add_circle(c, 4.0).unwrap();
    mirror(&mut sketch, &[circle], axis).unwrap();
    let copy = sketch
        .entities()
        .find(|(id, e)| *id != circle && matches!(e.geometry, Geometry::Circle { .. }))
        .map(|(id, _)| id)
        .unwrap();
    assert!(near(sketch.radius(copy).unwrap(), 4.0));
    let Some(Geometry::Circle { center, .. }) = sketch.entity(copy).map(|e| e.geometry.clone())
    else {
        panic!("not a circle");
    };
    let at = sketch.point(center).unwrap();
    assert!(near(at.x, -15.0) && near(at.y, 3.0));
    assert_eq!(count_of(&sketch, "equal"), 1);
    assert_eq!(count_of(&sketch, "symmetric"), 1);
}

#[test]
fn a_mirrored_arc_keeps_its_bulge_on_the_mirrored_side() {
    let mut sketch = Sketch::new();
    let axis = y_axis(&mut sketch);
    // A quarter arc counter-clockwise about (10, 0) from (15, 0) to (10, 5).
    let center = pt(&mut sketch, 10.0, 0.0);
    let start = pt(&mut sketch, 15.0, 0.0);
    let end = pt(&mut sketch, 10.0, 5.0);
    let arc = sketch.add_arc(center, start, end).unwrap();
    mirror(&mut sketch, &[arc], axis).unwrap();
    let copy = sketch
        .entities()
        .find(|(id, e)| *id != arc && matches!(e.geometry, Geometry::Arc { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let Some(Geometry::Arc { center, start, end }) =
        sketch.entity(copy).map(|e| e.geometry.clone())
    else {
        panic!("not an arc");
    };
    let (c, s, e) = (
        sketch.point(center).unwrap(),
        sketch.point(start).unwrap(),
        sketch.point(end).unwrap(),
    );
    // Counter-clockwise from (-10, 5) to (-15, 0): the quarter on the upper left.
    assert!(near(c.x, -10.0) && near(c.y, 0.0));
    assert!(near(s.x, -10.0) && near(s.y, 5.0));
    assert!(near(e.x, -15.0) && near(e.y, 0.0));
    // The centre and one end are held; the other end is implied by the circle.
    assert_eq!(count_of(&sketch, "symmetric"), 2);
}

#[test]
fn mirror_refusals_leave_the_sketch_as_it_was() {
    let mut sketch = Sketch::new();
    let axis = y_axis(&mut sketch);
    let on_axis = line(&mut sketch, (0.0, 1.0), (0.0, 9.0));
    let off = line(&mut sketch, (5.0, 1.0), (5.0, 9.0));
    let c = pt(&mut sketch, 5.0, 5.0);
    let drawn = sketch.clone();
    let cases: [(Vec<EntityId>, EntityId, &str); 6] = [
        (vec![], axis, "pick at least one"),
        (vec![off, off], axis, "twice"),
        (vec![axis], axis, "its own side"),
        (vec![on_axis], axis, "lies on the mirror line"),
        (vec![off], c, "line"),
        (vec![off], EntityId(999), "999"),
    ];
    for (entities, axis, expected) in cases {
        let message = mirror(&mut sketch, &entities, axis)
            .unwrap_err()
            .to_string();
        assert!(message.contains(expected), "{message}");
        assert_eq!(sketch, drawn);
    }
}

#[test]
fn a_linear_pattern_steps_copies_and_holds_them_equal() {
    let mut sketch = Sketch::new();
    let original = line(&mut sketch, (0.0, 0.0), (4.0, 0.0));
    let before = last_id(&sketch);
    pattern_linear(&mut sketch, &[original], 4, Point2::new(10.0, 2.0)).unwrap();
    let mut copies = lines_after(&sketch, before);
    copies.sort_by(|a, b| a.0.0.total_cmp(&b.0.0));
    assert_eq!(copies.len(), 3);
    for (n, copy) in copies.iter().enumerate() {
        let k = f64::from(u32::try_from(n).unwrap() + 1);
        assert!(near(copy.0.0, 10.0 * k) && near(copy.0.1, 2.0 * k));
        assert!(near(copy.1.0, 10.0 * k + 4.0));
    }
    assert_eq!(count_of(&sketch, "equal"), 3);
}

#[test]
fn a_linear_pattern_of_a_circle_keeps_its_radius() {
    let mut sketch = Sketch::new();
    let c = pt(&mut sketch, 0.0, 0.0);
    let circle = sketch.add_circle(c, 2.0).unwrap();
    pattern_linear(&mut sketch, &[circle], 3, Point2::new(0.0, 8.0)).unwrap();
    let radii: Vec<f64> = sketch
        .entities()
        .filter(|(_, e)| matches!(e.geometry, Geometry::Circle { .. }))
        .map(|(id, _)| sketch.radius(id).unwrap())
        .collect();
    assert_eq!(radii.len(), 3);
    assert!(radii.iter().all(|r| near(*r, 2.0)));
}

#[test]
fn a_circular_pattern_spreads_copies_round_a_full_turn() {
    let mut sketch = Sketch::new();
    let original = line(&mut sketch, (10.0, 0.0), (14.0, 0.0));
    let before = last_id(&sketch);
    pattern_circular(&mut sketch, &[original], 4, Point2::new(0.0, 0.0), 360.0).unwrap();
    let mut starts: Vec<(f64, f64)> = lines_after(&sketch, before).iter().map(|l| l.0).collect();
    starts.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(starts.len(), 3);
    // 90, 180 and 270 degrees at radius 10.
    assert!(near(starts[0].0, -10.0) && near(starts[0].1, 0.0));
    let (low, high) = if starts[1].1 < starts[2].1 {
        (starts[1], starts[2])
    } else {
        (starts[2], starts[1])
    };
    assert!(near(low.0, 0.0) && near(low.1, -10.0));
    assert!(near(high.0, 0.0) && near(high.1, 10.0));
}

#[test]
fn a_partial_circular_pattern_ends_on_the_angle() {
    let mut sketch = Sketch::new();
    let original = line(&mut sketch, (10.0, 0.0), (14.0, 0.0));
    let before = last_id(&sketch);
    pattern_circular(&mut sketch, &[original], 3, Point2::new(0.0, 0.0), 90.0).unwrap();
    let copies = lines_after(&sketch, before);
    assert_eq!(copies.len(), 2);
    // The last copy is turned a quarter: it starts at (0, 10).
    assert!(copies.iter().any(|l| near(l.0.0, 0.0) && near(l.0.1, 10.0)));
}

#[test]
fn pattern_refusals_leave_the_sketch_as_it_was() {
    let mut sketch = Sketch::new();
    let original = line(&mut sketch, (0.0, 0.0), (4.0, 0.0));
    let drawn = sketch.clone();
    let origin = Point2::new(0.0, 0.0);
    let step = Point2::new(5.0, 0.0);
    let results = [
        pattern_linear(&mut sketch, &[original], 1, step),
        pattern_linear(&mut sketch, &[original], 500, step),
        pattern_linear(&mut sketch, &[original], 3, origin),
        pattern_linear(&mut sketch, &[], 3, step),
        pattern_linear(&mut sketch, &[EntityId(999)], 3, step),
        pattern_circular(&mut sketch, &[original], 3, origin, 0.0),
        pattern_circular(&mut sketch, &[original], 3, origin, 400.0),
        pattern_circular(&mut sketch, &[original], 1, origin, 360.0),
    ];
    for result in results {
        assert!(result.is_err());
    }
    assert_eq!(sketch, drawn);
}

#[test]
fn the_commands_run_undo_and_redo_through_a_session() {
    let mut session = Session::new(Document::default());
    session
        .execute(Box::new(CreateSketch::new("", Plane::Xy)))
        .unwrap();
    let id = SketchId(0);
    let run = |session: &mut Session, edit: SketchEdit| {
        session.execute(Box::new(EditSketch::new(id, edit)))
    };
    run(
        &mut session,
        SketchEdit::AddLine {
            start: PointRef::New(Point2::new(5.0, 0.0)),
            end: PointRef::New(Point2::new(9.0, 0.0)),
        },
    )
    .unwrap();
    run(
        &mut session,
        SketchEdit::PatternLinear {
            entities: vec![EntityId(2)],
            count: 3,
            step: Point2::new(0.0, 5.0),
        },
    )
    .unwrap();
    let lines = |session: &Session| {
        session
            .document()
            .sketch(id)
            .unwrap()
            .sketch
            .entities()
            .filter(|(_, e)| matches!(e.geometry, Geometry::Line { .. }))
            .count()
    };
    assert_eq!(lines(&session), 3);
    session.undo().unwrap();
    assert_eq!(lines(&session), 1);
    session.redo().unwrap();
    assert_eq!(lines(&session), 3);
}
