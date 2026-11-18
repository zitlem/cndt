// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::Session;
use crate::sketch_commands::{CreateSketch, EditSketch, SketchEdit};
use cndt_model::{Document, Plane, SketchId};

type Segment = ((f64, f64), (f64, f64));

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn pt(sketch: &mut Sketch, x: f64, y: f64) -> EntityId {
    sketch.add_point(Point2::new(x, y)).unwrap()
}

/// Lines joined end to end through the given corners; closed if `close`.
fn polyline(sketch: &mut Sketch, corners: &[(f64, f64)], close: bool) -> Vec<EntityId> {
    let points: Vec<EntityId> = corners.iter().map(|c| pt(sketch, c.0, c.1)).collect();
    let mut lines: Vec<EntityId> = points
        .windows(2)
        .map(|w| sketch.add_line(w[0], w[1]).unwrap())
        .collect();
    if close {
        lines.push(sketch.add_line(*points.last().unwrap(), points[0]).unwrap());
    }
    lines
}

/// Every line with an id above `after`, as its two end points.
fn new_lines(sketch: &Sketch, after: u32) -> Vec<Segment> {
    sketch
        .entities()
        .filter(|(id, _)| id.0 > after)
        .filter_map(|(_, e)| match e.geometry {
            Geometry::Line { start, end } => {
                let (a, b) = (sketch.point(start)?, sketch.point(end)?);
                Some(((a.x, a.y), (b.x, b.y)))
            }
            _ => None,
        })
        .collect()
}

fn last_id(sketch: &Sketch) -> u32 {
    sketch.entities().map(|(id, _)| id.0).max().unwrap_or(0)
}

/// Smallest and largest x, then y, over all end points.
fn extent(lines: &[Segment]) -> (f64, f64, f64, f64) {
    let xs = lines.iter().flat_map(|l| [l.0.0, l.1.0]);
    let ys = lines.iter().flat_map(|l| [l.0.1, l.1.1]);
    (
        xs.clone().fold(f64::MAX, f64::min),
        xs.fold(f64::MIN, f64::max),
        ys.clone().fold(f64::MAX, f64::min),
        ys.fold(f64::MIN, f64::max),
    )
}

fn rectangle() -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new();
    let corners = [(0.0, 0.0), (40.0, 0.0), (40.0, 20.0), (0.0, 20.0)];
    let lines = polyline(&mut sketch, &corners, true);
    (sketch, lines)
}

#[test]
fn a_rectangle_offset_outward_grows_on_every_side() {
    let (mut sketch, lines) = rectangle();
    let before = last_id(&sketch);
    offset(&mut sketch, &lines, 2.0, Point2::new(-10.0, -10.0)).unwrap();
    let made = new_lines(&sketch, before);
    assert_eq!(made.len(), 4);
    let (x0, x1, y0, y1) = extent(&made);
    assert!(near(x0, -2.0) && near(x1, 42.0) && near(y0, -2.0) && near(y1, 22.0));
    // The originals stay.
    assert_eq!(new_lines(&sketch, 0).len(), 8);
}

#[test]
fn a_rectangle_offset_inward_shrinks_whichever_way_it_was_drawn() {
    // Drawn clockwise: "toward the middle" still means inward.
    let mut sketch = Sketch::new();
    let corners = [(0.0, 0.0), (0.0, 20.0), (40.0, 20.0), (40.0, 0.0)];
    let lines = polyline(&mut sketch, &corners, true);
    let before = last_id(&sketch);
    offset(&mut sketch, &lines, 3.0, Point2::new(20.0, 10.0)).unwrap();
    let (x0, x1, y0, y1) = extent(&new_lines(&sketch, before));
    assert!(near(x0, 3.0) && near(x1, 37.0) && near(y0, 3.0) && near(y1, 17.0));
}

#[test]
fn an_inward_offset_that_collapses_is_refused_and_changes_nothing() {
    for distance in [10.0, 12.0] {
        let (mut sketch, lines) = rectangle();
        let drawn = sketch.clone();
        let message = offset(&mut sketch, &lines, distance, Point2::new(20.0, 10.0))
            .unwrap_err()
            .to_string();
        assert!(message.contains("collapses"), "{message}");
        assert_eq!(sketch, drawn);
    }
}

#[test]
fn a_circle_offset_outward_and_inward() {
    let mut sketch = Sketch::new();
    let c = pt(&mut sketch, 5.0, 5.0);
    let circle = sketch.add_circle(c, 10.0).unwrap();
    offset(&mut sketch, &[circle], 4.0, Point2::new(50.0, 5.0)).unwrap();
    offset(&mut sketch, &[circle], 4.0, Point2::new(5.0, 6.0)).unwrap();
    let mut radii: Vec<f64> = sketch
        .entities()
        .filter_map(|(_, e)| match e.geometry {
            Geometry::Circle { radius, .. } => Some(radius),
            _ => None,
        })
        .collect();
    radii.sort_by(f64::total_cmp);
    assert_eq!(radii.len(), 3);
    assert!(near(radii[0], 6.0) && near(radii[1], 10.0) && near(radii[2], 14.0));
    let message = offset(&mut sketch, &[circle], 10.0, Point2::new(5.0, 6.0))
        .unwrap_err()
        .to_string();
    assert!(message.contains("leaves no circle"), "{message}");
}

/// Each arc as (radius, start, end).
type ArcShape = (f64, (f64, f64), (f64, f64));

fn arcs(sketch: &Sketch) -> Vec<ArcShape> {
    sketch
        .entities()
        .filter_map(|(_, e)| match e.geometry {
            Geometry::Arc { center, start, end } => {
                let (c, s, f) = (
                    sketch.point(center)?,
                    sketch.point(start)?,
                    sketch.point(end)?,
                );
                Some((c.distance(s), (s.x, s.y), (f.x, f.y)))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn an_arc_keeps_its_centre_and_changes_its_radius() {
    let mut sketch = Sketch::new();
    let (c, s, e) = (
        pt(&mut sketch, 0.0, 0.0),
        pt(&mut sketch, 10.0, 0.0),
        pt(&mut sketch, 0.0, 10.0),
    );
    let arc = sketch.add_arc(c, s, e).unwrap();
    offset(&mut sketch, &[arc], 3.0, Point2::new(1.0, 1.0)).unwrap();
    offset(&mut sketch, &[arc], 3.0, Point2::new(20.0, 20.0)).unwrap();
    let all = arcs(&sketch);
    assert_eq!(all.len(), 3);
    let (inner, outer) = (all[1], all[2]);
    assert!(near(inner.0, 7.0) && near(inner.1.0, 7.0) && near(inner.2.1, 7.0));
    assert!(near(outer.0, 13.0) && near(outer.1.0, 13.0) && near(outer.2.1, 13.0));
    let message = offset(&mut sketch, &[arc], 10.0, Point2::new(1.0, 1.0))
        .unwrap_err()
        .to_string();
    assert!(message.contains("leaves nothing"), "{message}");
}

#[test]
fn a_line_and_a_tangent_arc_offset_as_one_chain() {
    let mut sketch = Sketch::new();
    let (a, b) = (pt(&mut sketch, -10.0, 0.0), pt(&mut sketch, 10.0, 0.0));
    let line = sketch.add_line(a, b).unwrap();
    let (c, e) = (pt(&mut sketch, 10.0, 10.0), pt(&mut sketch, 20.0, 10.0));
    let arc = sketch.add_arc(c, b, e).unwrap();
    let before = last_id(&sketch);
    // Toward the side the arc curves to: just above the line. The side is
    // read from the picked curve nearest the point, here the line.
    offset(&mut sketch, &[arc, line], 2.0, Point2::new(0.0, 1.0)).unwrap();
    let lines = new_lines(&sketch, before);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(near(lines[0].0.0, -10.0) && near(lines[0].0.1, 2.0));
    assert!(near(lines[0].1.0, 10.0) && near(lines[0].1.1, 2.0));
    let new_arc = arcs(&sketch).pop().unwrap();
    assert!(near(new_arc.0, 8.0), "{new_arc:?}");
    assert!(near(new_arc.1.0, 10.0) && near(new_arc.1.1, 2.0));
    assert!(near(new_arc.2.0, 18.0) && near(new_arc.2.1, 10.0));
}

#[test]
fn an_open_corner_is_extended_to_meet() {
    let mut sketch = Sketch::new();
    let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
    let lines = polyline(&mut sketch, &corners, false);
    let before = last_id(&sketch);
    offset(&mut sketch, &lines, 2.0, Point2::new(5.0, 5.0)).unwrap();
    let mut made = new_lines(&sketch, before);
    made.sort_by(|a, b| a.0.0.total_cmp(&b.0.0));
    assert_eq!(made.len(), 2);
    assert!(near(made[0].0.0, 0.0) && near(made[0].0.1, 2.0));
    assert!(near(made[0].1.0, 8.0) && near(made[0].1.1, 2.0));
    assert!(near(made[1].0.0, 8.0) && near(made[1].1.1, 10.0));
}

#[test]
fn what_cannot_be_offset_is_refused_with_the_reason() {
    let (mut sketch, lines) = rectangle();
    let point = sketch
        .entities()
        .find(|(_, e)| matches!(e.geometry, Geometry::Point { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let c = pt(&mut sketch, 100.0, 100.0);
    let circle = sketch.add_circle(c, 5.0).unwrap();
    let separate = polyline(&mut sketch, &[(0.0, 50.0), (10.0, 50.0)], false);
    let toward = Point2::new(-5.0, -5.0);
    let nan = Point2::new(f64::NAN, 0.0);
    let cases: Vec<(Vec<EntityId>, f64, Point2, &str)> = vec![
        (vec![], 1.0, toward, "pick at least one"),
        (vec![point], 1.0, toward, "needs a line, arc or circle"),
        (vec![EntityId(999)], 1.0, toward, "does not exist"),
        (vec![lines[0], lines[0]], 1.0, toward, "picked twice"),
        (vec![lines[0], separate[0]], 1.0, toward, "not one chain"),
        (vec![lines[0], circle], 1.0, toward, "on its own"),
        (vec![lines[0]], 0.0, toward, "offset distance"),
        (vec![lines[0]], f64::NAN, toward, "offset distance"),
        (vec![lines[0]], 1.0, nan, "offset side"),
        (
            vec![lines[0]],
            1.0,
            Point2::new(20.0, 0.0),
            "lies on the curve",
        ),
        (
            vec![circle],
            1.0,
            Point2::new(105.0, 100.0),
            "lies on the circle",
        ),
    ];
    for (entities, distance, at, expected) in cases {
        let drawn = sketch.clone();
        let message = offset(&mut sketch, &entities, distance, at)
            .unwrap_err()
            .to_string();
        assert!(message.contains(expected), "{expected}: {message}");
        assert_eq!(sketch, drawn, "{expected}");
    }
}

#[test]
fn three_lines_at_one_point_are_a_branch() {
    let mut sketch = Sketch::new();
    let hub = pt(&mut sketch, 0.0, 0.0);
    let mut lines = Vec::new();
    for (x, y) in [(10.0, 0.0), (0.0, 10.0), (-10.0, 0.0)] {
        let end = pt(&mut sketch, x, y);
        lines.push(sketch.add_line(hub, end).unwrap());
    }
    let message = offset(&mut sketch, &lines, 1.0, Point2::new(1.0, 1.0))
        .unwrap_err()
        .to_string();
    assert!(message.contains("branch"), "{message}");
}

#[test]
fn offset_undo_and_redo_through_the_session() {
    let mut session = Session::new(Document::default());
    session
        .execute(Box::new(CreateSketch::new("", Plane::Xy)))
        .unwrap();
    let id = SketchId(0);
    let run = |session: &mut Session, edit: SketchEdit| {
        session.execute(Box::new(EditSketch::new(id, edit)))
    };
    let rectangle = SketchEdit::AddRectangle {
        from: Point2::new(0.0, 0.0),
        to: Point2::new(40.0, 20.0),
    };
    run(&mut session, rectangle).unwrap();
    let current = |session: &Session| session.document().sketch(id).unwrap().sketch.clone();
    let lines: Vec<EntityId> = current(&session)
        .entities()
        .filter(|(_, e)| matches!(e.geometry, Geometry::Line { .. }))
        .map(|(id, _)| id)
        .collect();
    let before = current(&session);
    let edit = SketchEdit::Offset {
        entities: lines,
        distance: 2.0,
        toward: Point2::new(-5.0, -5.0),
    };
    run(&mut session, edit).unwrap();
    let after = current(&session);
    assert_ne!(before, after);
    assert_eq!(session.undo_label().as_deref(), Some("Offset"));
    session.undo().unwrap();
    assert_eq!(current(&session), before);
    session.redo().unwrap();
    assert_eq!(current(&session), after);
}
