// SPDX-License-Identifier: AGPL-3.0-or-later
//! Solver tests, built on the sketches of the CNDT test block (decision 0010).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use super::*;

const TOL: f64 = 1e-7;

fn opts() -> SolveOptions {
    SolveOptions::default()
}

fn pt(s: &mut Sketch, x: f64, y: f64) -> EntityId {
    s.add_point(Point2::new(x, y)).unwrap()
}

fn con(s: &mut Sketch, c: Constraint) -> ConstraintId {
    let kind = c.kind();
    s.add_constraint(c, &opts())
        .unwrap_or_else(|e| panic!("{kind}: {e}"))
}

fn at(s: &Sketch, id: EntityId) -> Point2 {
    s.point(id).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < TOL
}

fn assert_at(s: &Sketch, id: EntityId, x: f64, y: f64) {
    let p = at(s, id);
    assert!(
        close(p.x, x) && close(p.y, y),
        "{id} is at ({}, {}), expected ({x}, {y})",
        p.x,
        p.y
    );
}

/// A 96 × 56 rectangle from rough corners, its bottom-left corner fixed.
struct Rect {
    s: Sketch,
    corners: [EntityId; 4],
    bottom: EntityId,
    right: EntityId,
    top: EntityId,
    left: EntityId,
    width: ConstraintId,
}

fn rectangle() -> Rect {
    let mut s = Sketch::new();
    let p0 = pt(&mut s, 0.0, 0.0);
    let p1 = pt(&mut s, 90.0, 1.0);
    let p2 = pt(&mut s, 92.0, 50.0);
    let p3 = pt(&mut s, -1.0, 52.0);
    let bottom = s.add_line(p0, p1).unwrap();
    let right = s.add_line(p1, p2).unwrap();
    let top = s.add_line(p2, p3).unwrap();
    let left = s.add_line(p3, p0).unwrap();
    con(&mut s, Constraint::Fix { point: p0 });
    con(&mut s, Constraint::Horizontal { line: bottom });
    con(&mut s, Constraint::Horizontal { line: top });
    con(&mut s, Constraint::Vertical { line: left });
    con(&mut s, Constraint::Vertical { line: right });
    let width = con(
        &mut s,
        Constraint::Length {
            line: bottom,
            value: 96.0,
        },
    );
    con(
        &mut s,
        Constraint::Length {
            line: left,
            value: 56.0,
        },
    );
    Rect {
        s,
        corners: [p0, p1, p2, p3],
        bottom,
        right,
        top,
        left,
        width,
    }
}

#[test]
fn unconstrained_entities_have_their_own_freedom() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    let c = pt(&mut s, 30.0, 0.0);
    let circle = s.add_circle(c, 5.0).unwrap();
    let (ac, a1, a2) = (
        pt(&mut s, 50.0, 0.0),
        pt(&mut s, 55.0, 0.0),
        pt(&mut s, 50.0, 5.0),
    );
    let arc = s.add_arc(ac, a1, a2).unwrap();
    let f = s.freedom();
    let dof = |id| f.entities.iter().find(|e| e.id == id).unwrap().dof;
    assert_eq!(dof(a), Some(2));
    assert_eq!(dof(line), Some(4));
    assert_eq!(dof(circle), Some(3));
    assert_eq!(dof(arc), Some(5), "an arc keeps its end on its circle");
    // 2 + 2 + (2 + 1) + (6 − 1)
    assert_eq!(f.dof, Some(12));
    assert_eq!(f.status, Status::Under);
}

#[test]
fn rectangle_is_fully_constrained_and_exact() {
    let r = rectangle();
    let [p0, p1, p2, p3] = r.corners;
    assert_at(&r.s, p0, 0.0, 0.0);
    assert_at(&r.s, p1, 96.0, 0.0);
    assert_at(&r.s, p2, 96.0, 56.0);
    assert_at(&r.s, p3, 0.0, 56.0);
    let f = r.s.freedom();
    assert_eq!((f.dof, f.redundant, f.status), (Some(0), 0, Status::Fully));
    assert!(f.entities.iter().all(|e| e.status == Status::Fully));
}

#[test]
fn freedom_counts_down_as_constraints_are_added() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 10.0, 1.0);
    let line = s.add_line(a, b).unwrap();
    assert_eq!(s.freedom().dof, Some(4));
    con(&mut s, Constraint::Fix { point: a });
    assert_eq!(s.freedom().dof, Some(2));
    con(&mut s, Constraint::Horizontal { line });
    assert_eq!(s.freedom().dof, Some(1));
    con(&mut s, Constraint::Length { line, value: 25.0 });
    let f = s.freedom();
    assert_eq!((f.dof, f.status), (Some(0), Status::Fully));
    assert_at(&s, b, 25.0, 0.0);
}

#[test]
fn a_conflicting_dimension_is_refused_and_named() {
    let mut r = rectangle();
    let before = r.s.clone();
    let error = r.s.add_constraint(
        Constraint::Length {
            line: r.top,
            value: 90.0,
        },
        &opts(),
    );
    let Err(SketchError::Conflict { constraints }) = error else {
        panic!("expected a conflict, got {error:?}");
    };
    let new = ConstraintId(7);
    assert_eq!(
        constraints.first(),
        Some(&new),
        "the new constraint is named first"
    );
    assert!(
        constraints.contains(&r.width),
        "the 96 mm width is named: {constraints:?}"
    );
    assert_eq!(r.s, before, "a refused constraint changes nothing");
}

#[test]
fn a_redundant_constraint_is_refused_and_named() {
    let mut r = rectangle();
    let before = r.s.clone();
    let error = r.s.add_constraint(
        Constraint::Parallel {
            a: r.top,
            b: r.bottom,
        },
        &opts(),
    );
    let Err(SketchError::Redundant { constraints }) = error else {
        panic!("expected redundancy, got {error:?}");
    };
    // Horizontal top and horizontal bottom already make them parallel.
    assert!(constraints.contains(&ConstraintId(1)) && constraints.contains(&ConstraintId(2)));
    assert_eq!(r.s, before);
    // Fixing an already fixed point is redundant too.
    let error = r.s.add_constraint(
        Constraint::Fix {
            point: r.corners[0],
        },
        &opts(),
    );
    assert!(
        matches!(error, Err(SketchError::Redundant { .. })),
        "{error:?}"
    );
    // So is fixing a point the other constraints already pin down.
    let error = r.s.add_constraint(
        Constraint::Fix {
            point: r.corners[2],
        },
        &opts(),
    );
    assert!(
        matches!(error, Err(SketchError::Redundant { .. })),
        "{error:?}"
    );
}

#[test]
fn changing_a_dimension_moves_only_what_it_must() {
    let mut r = rectangle();
    r.s.set_dimension(r.width, 100.0, &opts()).unwrap();
    let [p0, p1, p2, p3] = r.corners;
    assert_at(&r.s, p0, 0.0, 0.0);
    assert_at(&r.s, p1, 100.0, 0.0);
    assert_at(&r.s, p2, 100.0, 56.0);
    assert_at(&r.s, p3, 0.0, 56.0);
    let before = r.s.clone();
    let error = r.s.set_dimension(r.width, -5.0, &opts());
    assert!(
        matches!(error, Err(SketchError::BadValue { .. })),
        "{error:?}"
    );
    let error = r.s.set_dimension(ConstraintId(1), 5.0, &opts());
    assert_eq!(error, Err(SketchError::NotADimension(ConstraintId(1))));
    assert_eq!(r.s, before);
    let _ = (r.right, r.left);
}

/// The test block's outside profile: 96 × 56 with R5 corners (decision 0010,
/// feature 2), drawn roughly and solved exactly.
struct Outline {
    s: Sketch,
    centers: [EntityId; 4],
    right: EntityId,
    top: EntityId,
}

fn outline(noise: f64) -> Outline {
    let mut s = Sketch::new();
    let n = |k: f64| noise * (k * 1.7).sin();
    let origin = pt(&mut s, 0.0, 0.0);
    // Line end points, counter-clockwise from the bottom edge.
    let b0 = pt(&mut s, 5.0 + n(1.0), 0.0 + n(2.0));
    let b1 = pt(&mut s, 91.0 + n(3.0), 0.0 + n(4.0));
    let r0 = pt(&mut s, 96.0 + n(5.0), 5.0 + n(6.0));
    let r1 = pt(&mut s, 96.0 + n(7.0), 51.0 + n(8.0));
    let t0 = pt(&mut s, 91.0 + n(9.0), 56.0 + n(10.0));
    let t1 = pt(&mut s, 5.0 + n(11.0), 56.0 + n(12.0));
    let l0 = pt(&mut s, 0.0 + n(13.0), 51.0 + n(14.0));
    let l1 = pt(&mut s, 0.0 + n(15.0), 5.0 + n(16.0));
    let c_br = pt(&mut s, 91.0 + n(17.0), 5.0 + n(18.0));
    let c_tr = pt(&mut s, 91.0 + n(19.0), 51.0 + n(20.0));
    let c_tl = pt(&mut s, 5.0 + n(21.0), 51.0 + n(22.0));
    let c_bl = pt(&mut s, 5.0 + n(23.0), 5.0 + n(24.0));
    let bottom = s.add_line(b0, b1).unwrap();
    let right = s.add_line(r0, r1).unwrap();
    let top = s.add_line(t0, t1).unwrap();
    let left = s.add_line(l0, l1).unwrap();
    let br = s.add_arc(c_br, b1, r0).unwrap();
    let tr = s.add_arc(c_tr, r1, t0).unwrap();
    let tl = s.add_arc(c_tl, t1, l0).unwrap();
    let bl = s.add_arc(c_bl, l1, b0).unwrap();
    con(&mut s, Constraint::Fix { point: origin });
    con(
        &mut s,
        Constraint::PointOnLine {
            point: origin,
            line: bottom,
        },
    );
    con(
        &mut s,
        Constraint::PointOnLine {
            point: origin,
            line: left,
        },
    );
    con(&mut s, Constraint::Horizontal { line: bottom });
    con(&mut s, Constraint::Horizontal { line: top });
    con(&mut s, Constraint::Vertical { line: left });
    con(&mut s, Constraint::Vertical { line: right });
    for (line, arc) in [
        (bottom, br),
        (right, br),
        (right, tr),
        (top, tr),
        (top, tl),
        (left, tl),
        (left, bl),
        (bottom, bl),
    ] {
        con(&mut s, Constraint::Tangent { a: line, b: arc });
    }
    con(&mut s, Constraint::Equal { a: br, b: tr });
    con(&mut s, Constraint::Equal { a: br, b: tl });
    con(&mut s, Constraint::Equal { a: br, b: bl });
    con(
        &mut s,
        Constraint::Radius {
            curve: br,
            value: 5.0,
        },
    );
    con(
        &mut s,
        Constraint::Distance {
            a: origin,
            b: r0,
            axis: Axis::Horizontal,
            value: 96.0,
        },
    );
    con(
        &mut s,
        Constraint::Distance {
            a: origin,
            b: t0,
            axis: Axis::Vertical,
            value: 56.0,
        },
    );
    Outline {
        s,
        centers: [c_bl, c_br, c_tr, c_tl],
        right,
        top,
    }
}

#[test]
fn outside_profile_with_corner_radii_solves_exactly() {
    let o = outline(0.3);
    let [bl, br, tr, tl] = o.centers;
    assert_at(&o.s, bl, 5.0, 5.0);
    assert_at(&o.s, br, 91.0, 5.0);
    assert_at(&o.s, tr, 91.0, 51.0);
    assert_at(&o.s, tl, 5.0, 51.0);
    let Some(Geometry::Line { start, end }) = o.s.entity(o.right).map(|e| e.geometry.clone())
    else {
        panic!("right edge");
    };
    assert_at(&o.s, start, 96.0, 5.0);
    assert_at(&o.s, end, 96.0, 51.0);
    let Some(Geometry::Line { start, .. }) = o.s.entity(o.top).map(|e| e.geometry.clone()) else {
        panic!("top edge");
    };
    assert_at(&o.s, start, 91.0, 56.0);
    let f = o.s.freedom();
    assert_eq!(
        (f.dof, f.redundant, f.status),
        (Some(0), 0, Status::Fully),
        "{f:?}"
    );
}

#[test]
fn solving_is_deterministic() {
    let bits = |s: &Sketch| -> Vec<(u64, u64)> {
        s.entities()
            .filter_map(|(id, _)| s.point(id))
            .map(|p| (p.x.to_bits(), p.y.to_bits()))
            .collect()
    };
    assert_eq!(bits(&outline(0.3).s), bits(&outline(0.3).s));
}

/// Closed slot: 30 mm long, 8 mm wide (decision 0010, feature 4).
#[test]
fn closed_slot_is_fully_constrained() {
    let mut s = Sketch::new();
    let left_c = pt(&mut s, 0.0, 0.0);
    let right_c = pt(&mut s, 21.0, 0.5);
    let (r_lo, r_hi) = (pt(&mut s, 21.5, -4.2), pt(&mut s, 21.0, 4.3));
    let (l_hi, l_lo) = (pt(&mut s, 0.3, 3.8), pt(&mut s, -0.2, -4.1));
    let right = s.add_arc(right_c, r_lo, r_hi).unwrap();
    let left = s.add_arc(left_c, l_hi, l_lo).unwrap();
    let top = s.add_line(r_hi, l_hi).unwrap();
    let bottom = s.add_line(l_lo, r_lo).unwrap();
    con(&mut s, Constraint::Fix { point: left_c });
    con(
        &mut s,
        Constraint::HorizontalPoints {
            a: left_c,
            b: right_c,
        },
    );
    for (line, arc) in [(top, right), (top, left), (bottom, right), (bottom, left)] {
        con(&mut s, Constraint::Tangent { a: line, b: arc });
    }
    con(&mut s, Constraint::Equal { a: left, b: right });
    con(
        &mut s,
        Constraint::Diameter {
            curve: left,
            value: 8.0,
        },
    );
    // Overall length 30 = centre distance + width.
    con(
        &mut s,
        Constraint::Distance {
            a: left_c,
            b: right_c,
            axis: Axis::Aligned,
            value: 22.0,
        },
    );
    assert_at(&s, right_c, 22.0, 0.0);
    assert_at(&s, r_hi, 22.0, 4.0);
    assert_at(&s, l_hi, 0.0, 4.0);
    assert_at(&s, l_lo, 0.0, -4.0);
    assert!(close(s.radius(right).unwrap(), 4.0));
    let f = s.freedom();
    assert_eq!((f.dof, f.status), (Some(0), Status::Fully), "{f:?}");
}

/// A Ø6.6 clearance hole placed from the origin (decision 0010, feature 7).
#[test]
fn hole_positioned_from_origin() {
    let mut s = Sketch::new();
    let origin = pt(&mut s, 0.0, 0.0);
    let c = pt(&mut s, 11.0, 7.0);
    let hole = s.add_circle(c, 2.0).unwrap();
    con(&mut s, Constraint::Fix { point: origin });
    con(
        &mut s,
        Constraint::Diameter {
            curve: hole,
            value: 6.6,
        },
    );
    con(
        &mut s,
        Constraint::Distance {
            a: origin,
            b: c,
            axis: Axis::Horizontal,
            value: 10.0,
        },
    );
    assert_eq!(s.freedom().dof, Some(1));
    con(
        &mut s,
        Constraint::Distance {
            a: origin,
            b: c,
            axis: Axis::Vertical,
            value: 8.0,
        },
    );
    assert_at(&s, c, 10.0, 8.0);
    assert!(close(s.radius(hole).unwrap(), 3.3));
    assert_eq!(s.freedom().status, Status::Fully);
}

#[test]
fn signed_distances_keep_their_side() {
    // A point left of the origin stays left when dimensioned.
    let mut s = Sketch::new();
    let origin = pt(&mut s, 0.0, 0.0);
    let p = pt(&mut s, -7.0, 0.0);
    con(&mut s, Constraint::Fix { point: origin });
    con(
        &mut s,
        Constraint::Distance {
            a: origin,
            b: p,
            axis: Axis::Horizontal,
            value: 12.0,
        },
    );
    assert!(close(at(&s, p).x, -12.0));
}

#[test]
fn angle_between_lines() {
    let mut s = Sketch::new();
    let corner = pt(&mut s, 0.0, 0.0);
    let a = pt(&mut s, 20.0, 0.0);
    let b = pt(&mut s, 15.0, 13.0);
    let base = s.add_line(corner, a).unwrap();
    let arm = s.add_line(corner, b).unwrap();
    con(&mut s, Constraint::Fix { point: corner });
    con(&mut s, Constraint::Horizontal { line: base });
    con(
        &mut s,
        Constraint::Angle {
            a: base,
            b: arm,
            value: 30.0,
        },
    );
    let p = at(&s, b);
    assert!(close(p.y.atan2(p.x), 30f64.to_radians()), "{p:?}");
    // Drawn the other way round (towards the corner), still 30°, not 150°.
    let mut s = Sketch::new();
    let corner = pt(&mut s, 0.0, 0.0);
    let a = pt(&mut s, 20.0, 0.0);
    let b = pt(&mut s, 15.0, 13.0);
    let base = s.add_line(a, corner).unwrap();
    let arm = s.add_line(b, corner).unwrap();
    con(&mut s, Constraint::Fix { point: corner });
    con(&mut s, Constraint::Horizontal { line: base });
    con(
        &mut s,
        Constraint::Angle {
            a: base,
            b: arm,
            value: 30.0,
        },
    );
    let p = at(&s, b);
    assert!(close(p.y.atan2(p.x), 30f64.to_radians()), "{p:?}");
}

#[test]
fn tangent_circles_touch() {
    let mut s = Sketch::new();
    let (c1, c2) = (pt(&mut s, 0.0, 0.0), pt(&mut s, 18.0, 0.0));
    let a = s.add_circle(c1, 5.0).unwrap();
    let b = s.add_circle(c2, 8.0).unwrap();
    con(&mut s, Constraint::Fix { point: c1 });
    con(
        &mut s,
        Constraint::Radius {
            curve: a,
            value: 5.0,
        },
    );
    con(
        &mut s,
        Constraint::Radius {
            curve: b,
            value: 8.0,
        },
    );
    con(&mut s, Constraint::Tangent { a, b });
    assert!(close(at(&s, c1).distance(at(&s, c2)), 13.0));
}

#[test]
fn midpoint_symmetric_concentric_collinear_perpendicular() {
    let mut s = Sketch::new();
    let (a, b) = (pt(&mut s, 0.0, 0.0), pt(&mut s, 10.0, 0.0));
    let line = s.add_line(a, b).unwrap();
    con(&mut s, Constraint::Fix { point: a });
    con(&mut s, Constraint::Horizontal { line });
    let m = pt(&mut s, 4.0, 1.0);
    con(&mut s, Constraint::Midpoint { point: m, line });
    let mid = at(&s, m);
    let (pa, pb) = (at(&s, a), at(&s, b));
    assert!(close(mid.x, (pa.x + pb.x) / 2.0) && close(mid.y, (pa.y + pb.y) / 2.0));

    let (u, v) = (pt(&mut s, 3.0, 4.0), pt(&mut s, 6.0, -3.0));
    con(
        &mut s,
        Constraint::Symmetric {
            a: u,
            b: v,
            axis: line,
        },
    );
    let (pu, pv) = (at(&s, u), at(&s, v));
    assert!(close(pu.x, pv.x) && close(pu.y, -pv.y));

    let (c1, c2) = (pt(&mut s, 30.0, 0.0), pt(&mut s, 31.0, 1.0));
    let k1 = s.add_circle(c1, 2.0).unwrap();
    let k2 = s.add_circle(c2, 4.0).unwrap();
    con(&mut s, Constraint::Concentric { a: k1, b: k2 });
    assert!(close(at(&s, c1).distance(at(&s, c2)), 0.0));

    let (d, e) = (pt(&mut s, 20.0, 0.5), pt(&mut s, 25.0, -0.5));
    let other = s.add_line(d, e).unwrap();
    con(&mut s, Constraint::Collinear { a: line, b: other });
    assert!(close(at(&s, d).y, 0.0) && close(at(&s, e).y, 0.0));

    let (f, g) = (pt(&mut s, 40.0, 0.0), pt(&mut s, 41.0, 9.0));
    let upright = s.add_line(f, g).unwrap();
    con(
        &mut s,
        Constraint::Perpendicular {
            a: line,
            b: upright,
        },
    );
    assert!(close(at(&s, f).x, at(&s, g).x));
}

#[test]
fn dragging_a_point_keeps_the_constraints() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    con(&mut s, Constraint::Fix { point: a });
    con(&mut s, Constraint::Length { line, value: 10.0 });
    s.move_point(b, Point2::new(0.0, 20.0)).unwrap();
    let p = at(&s, b);
    assert!(
        close(p.x, 0.0) && close(p.y, 10.0),
        "slides round to (0, 10): {p:?}"
    );
    // A fixed point cannot be dragged.
    assert_eq!(
        s.move_point(a, Point2::new(5.0, 5.0)),
        Err(SketchError::Fixed(a))
    );
    assert_at(&s, a, 0.0, 0.0);
}

#[test]
fn malformed_constraints_explain_themselves() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 1.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    let c = pt(&mut s, 5.0, 5.0);
    let circle = s.add_circle(c, 1.0).unwrap();
    let wrong = s.add_constraint(Constraint::Horizontal { line: a }, &opts());
    assert_eq!(
        wrong,
        Err(SketchError::WrongGeometry {
            entity: a,
            found: "point",
            expected: "a line"
        })
    );
    assert_eq!(
        wrong.unwrap_err().to_string(),
        "entity 0 is a point, but this needs a line"
    );
    let same = s.add_constraint(Constraint::Parallel { a: line, b: line }, &opts());
    assert_eq!(same, Err(SketchError::SameEntity(line)));
    let zero = s.add_constraint(
        Constraint::Radius {
            curve: circle,
            value: 0.0,
        },
        &opts(),
    );
    assert!(matches!(zero, Err(SketchError::BadValue { .. })));
    let flat = s.add_constraint(
        Constraint::Angle {
            a: line,
            b: line,
            value: 0.0,
        },
        &opts(),
    );
    assert!(matches!(flat, Err(SketchError::SameEntity(_))));
    let missing = s.add_constraint(
        Constraint::Fix {
            point: EntityId(99),
        },
        &opts(),
    );
    assert_eq!(missing, Err(SketchError::UnknownEntity(EntityId(99))));
    assert_eq!(s.constraints().count(), 0);
    assert!(matches!(
        s.add_circle(c, -1.0),
        Err(SketchError::BadValue { .. })
    ));
    assert!(matches!(s.add_line(a, a), Err(SketchError::SameEntity(_))));
    assert!(matches!(
        s.add_line(a, line),
        Err(SketchError::WrongGeometry { .. })
    ));
}

#[test]
fn removing_entities_and_constraints() {
    let mut r = rectangle();
    let [p0, ..] = r.corners;
    assert!(matches!(
        r.s.remove_entity(p0),
        Err(SketchError::InUse { .. })
    ));
    let removed = r.s.remove_entity(r.bottom).unwrap();
    assert_eq!(removed.len(), 2, "horizontal and the width");
    assert_eq!(
        r.s.freedom().dof,
        Some(2),
        "the bottom-right corner is free now"
    );
    r.s.remove_constraint(ConstraintId(0)).unwrap();
    assert_eq!(
        r.s.remove_constraint(ConstraintId(0)),
        Err(SketchError::UnknownConstraint(ConstraintId(0)))
    );
}

#[test]
fn sketches_round_trip_through_json() {
    let o = outline(0.0);
    let text = serde_json::to_string(&o.s).unwrap();
    let back: Sketch = serde_json::from_str(&text).unwrap();
    assert_eq!(back, o.s);
    assert!(text.contains(r#""type":"tangent""#), "{text}");
}

#[test]
fn over_constrained_sketch_reports_redundancy_after_loading() {
    // A file could hold redundant constraints that were never checked.
    let mut r = rectangle();
    r.s.constraints.insert(
        ConstraintId(50),
        Constraint::Parallel {
            a: r.top,
            b: r.bottom,
        },
    );
    let f = r.s.freedom();
    assert_eq!(f.status, Status::Over);
    assert_eq!(f.redundant, 1);
}

#[test]
fn line_tangent_to_a_separate_circle() {
    let mut s = Sketch::new();
    let (a, b) = (pt(&mut s, 0.0, 0.0), pt(&mut s, 30.0, 0.0));
    let floor = s.add_line(a, b).unwrap();
    let c = pt(&mut s, 12.0, 7.0);
    let circle = s.add_circle(c, 4.0).unwrap();
    con(&mut s, Constraint::Fix { point: a });
    con(&mut s, Constraint::Horizontal { line: floor });
    con(
        &mut s,
        Constraint::Radius {
            curve: circle,
            value: 4.0,
        },
    );
    con(
        &mut s,
        Constraint::Tangent {
            a: floor,
            b: circle,
        },
    );
    assert!(
        close(at(&s, c).y - at(&s, a).y, 4.0),
        "sits on the line, on its own side"
    );
}

#[test]
fn arcs_joined_end_to_end_stay_tangent() {
    // An S-bend: two R10 arcs meeting at the origin.
    let mut s = Sketch::new();
    let join = pt(&mut s, 0.0, 0.0);
    let (c1, s1) = (pt(&mut s, -10.0, 0.5), pt(&mut s, -10.0, -10.0));
    let (c2, e2) = (pt(&mut s, 10.0, -0.5), pt(&mut s, 10.0, 10.0));
    let first = s.add_arc(c1, s1, join).unwrap();
    let second = s.add_arc(c2, e2, join).unwrap();
    con(&mut s, Constraint::Fix { point: join });
    con(
        &mut s,
        Constraint::Tangent {
            a: first,
            b: second,
        },
    );
    let (p1, p2) = (at(&s, c1), at(&s, c2));
    // Both centres on one line through the joint.
    assert!((p1.x * p2.y - p1.y * p2.x).abs() < 1e-6, "{p1:?} {p2:?}");
}
