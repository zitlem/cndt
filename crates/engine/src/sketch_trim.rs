// SPDX-License-Identifier: AGPL-3.0-or-later
//! Trim: remove the part of a line, arc or circle between the places where
//! other curves cross it.
//!
//! Written from the textbook formulas for line and circle intersection; the
//! method is listed in `docs/provenance.md`.

use std::f64::consts::TAU;

use cndt_model::sketch::{Constraint, EntityId, Geometry, Point2, SolveOptions};
use cndt_model::{Sketch, SketchError};

/// Closer than this (millimetres) counts as the same place.
const NEAR: f64 = 1e-7;

/// A curve with its numbers read out of the sketch.
#[derive(Debug, Clone, Copy)]
enum Curve {
    Line {
        a: Point2,
        b: Point2,
    },
    Arc {
        center: Point2,
        radius: f64,
        /// Angle of the start, and how far it runs counter-clockwise.
        from: f64,
        sweep: f64,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
}

fn curve(sketch: &Sketch, id: EntityId) -> Option<Curve> {
    match sketch.entity(id)?.geometry {
        Geometry::Point { .. } => None,
        Geometry::Line { start, end } => Some(Curve::Line {
            a: sketch.point(start)?,
            b: sketch.point(end)?,
        }),
        Geometry::Circle { center, radius } => Some(Curve::Circle {
            center: sketch.point(center)?,
            radius,
        }),
        Geometry::Arc { center, start, end } => {
            let (c, s, e) = (
                sketch.point(center)?,
                sketch.point(start)?,
                sketch.point(end)?,
            );
            let from = (s.y - c.y).atan2(s.x - c.x);
            let to = (e.y - c.y).atan2(e.x - c.x);
            Some(Curve::Arc {
                center: c,
                radius: c.distance(s),
                from,
                sweep: (to - from).rem_euclid(TAU),
            })
        }
    }
}

/// Whether `angle` lies on an arc, with a little slack at both ends.
fn in_arc(angle: f64, from: f64, sweep: f64) -> bool {
    let offset = (angle - from).rem_euclid(TAU);
    offset <= sweep + 1e-9 || offset >= TAU - 1e-9
}

/// Where a line (as `a + t (b - a)`, `t` in 0..=1) meets a circle.
fn line_circle(a: Point2, b: Point2, center: Point2, radius: f64) -> Vec<Point2> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (fx, fy) = (a.x - center.x, a.y - center.y);
    let qa = dx * dx + dy * dy;
    let qb = 2.0 * (fx * dx + fy * dy);
    let qc = fx * fx + fy * fy - radius * radius;
    if qa <= 0.0 {
        return Vec::new();
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return Vec::new();
    }
    let root = disc.sqrt();
    let mut hits = Vec::new();
    for t in [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)] {
        let slack = NEAR / qa.sqrt();
        if t >= -slack && t <= 1.0 + slack {
            let p = Point2::new(a.x + t * dx, a.y + t * dy);
            if hits.iter().all(|h: &Point2| h.distance(p) > NEAR) {
                hits.push(p);
            }
        }
    }
    hits
}

pub(crate) fn circle_circle(c1: Point2, r1: f64, c2: Point2, r2: f64) -> Vec<Point2> {
    let d = c1.distance(c2);
    if d <= NEAR || d > r1 + r2 || d < (r1 - r2).abs() {
        return Vec::new();
    }
    let along = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
    let height = (r1 * r1 - along * along).max(0.0).sqrt();
    let (ux, uy) = ((c2.x - c1.x) / d, (c2.y - c1.y) / d);
    let (mx, my) = (c1.x + ux * along, c1.y + uy * along);
    let mut hits = vec![Point2::new(mx - uy * height, my + ux * height)];
    if height > NEAR {
        hits.push(Point2::new(mx + uy * height, my - ux * height));
    }
    hits
}

fn line_line(a: Point2, b: Point2, c: Point2, d: Point2) -> Vec<Point2> {
    let (rx, ry) = (b.x - a.x, b.y - a.y);
    let (sx, sy) = (d.x - c.x, d.y - c.y);
    let denominator = rx * sy - ry * sx;
    if denominator.abs() <= 1e-12 {
        return Vec::new();
    }
    let t = ((c.x - a.x) * sy - (c.y - a.y) * sx) / denominator;
    let u = ((c.x - a.x) * ry - (c.y - a.y) * rx) / denominator;
    let (slack_t, slack_u) = (NEAR / rx.hypot(ry).max(NEAR), NEAR / sx.hypot(sy).max(NEAR));
    if t >= -slack_t && t <= 1.0 + slack_t && u >= -slack_u && u <= 1.0 + slack_u {
        vec![Point2::new(a.x + t * rx, a.y + t * ry)]
    } else {
        Vec::new()
    }
}

/// The points where two curves meet.
fn meet(target: Curve, other: Curve) -> Vec<Point2> {
    // Arcs and circles are met as full circles, then cut down to the arc.
    let circle_of = |c: Curve| match c {
        Curve::Arc { center, radius, .. } | Curve::Circle { center, radius } => {
            Some((center, radius))
        }
        Curve::Line { .. } => None,
    };
    let arc_of = |c: Curve| match c {
        Curve::Arc { from, sweep, .. } => Some((from, sweep)),
        _ => None,
    };
    let mut hits = match (target, other) {
        (Curve::Line { a, b }, Curve::Line { a: c, b: d }) => line_line(a, b, c, d),
        (Curve::Line { a, b }, o) => circle_of(o)
            .map(|(center, radius)| line_circle(a, b, center, radius))
            .unwrap_or_default(),
        (t, Curve::Line { a, b }) => circle_of(t)
            .map(|(center, radius)| line_circle(a, b, center, radius))
            .unwrap_or_default(),
        (t, o) => match (circle_of(t), circle_of(o)) {
            (Some((c1, r1)), Some((c2, r2))) => circle_circle(c1, r1, c2, r2),
            _ => Vec::new(),
        },
    };
    // A hit must lie on both curves; the line-against-circle and
    // circle-against-circle formulas only checked the line's extent.
    for curve in [target, other] {
        if let (Some((center, _)), Some((from, sweep))) = (circle_of(curve), arc_of(curve)) {
            hits.retain(|p| in_arc((p.y - center.y).atan2(p.x - center.x), from, sweep));
        }
    }
    hits
}

/// Where along the target a point is: `0..=1` on a line, an offset in
/// radians from the start on an arc or circle.
fn parameter(target: Curve, p: Point2) -> f64 {
    match target {
        Curve::Line { a, b } => {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            ((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)
        }
        Curve::Arc { center, from, .. } => {
            ((p.y - center.y).atan2(p.x - center.x) - from).rem_euclid(TAU)
        }
        Curve::Circle { center, .. } => (p.y - center.y).atan2(p.x - center.x).rem_euclid(TAU),
    }
}

/// How long one unit of parameter is, to compare against `NEAR`.
fn scale(target: Curve) -> f64 {
    match target {
        Curve::Line { a, b } => a.distance(b),
        Curve::Arc { radius, .. } | Curve::Circle { radius, .. } => radius,
    }
}

/// A cut: its place on the target and the existing point already there, if
/// any, so a corner stays joined.
#[derive(Debug, Clone, Copy)]
struct Cut {
    at: f64,
    place: Point2,
    point: Option<EntityId>,
    /// The curve that makes the cut, which the new point is held on.
    cutter: EntityId,
}

fn error(what: String) -> SketchError {
    SketchError::Corrupt(what)
}

/// Trims `entity` at `click`: removes the stretch of it that contains the
/// point nearest `click`, between the nearest cuts either side.
///
/// Other curves cut it where they cross it (construction curves do not).
/// The pieces left reuse the entity's own end points, and a cut that falls
/// on a point already in the sketch uses that point. Constraints on the
/// trimmed entity go with it; each new end point is held on the curve that
/// cut it.
pub(crate) fn trim(
    sketch: &mut Sketch,
    entity: EntityId,
    click: Point2,
    options: &SolveOptions,
) -> Result<(), SketchError> {
    let target = curve(sketch, entity).ok_or_else(|| match sketch.entity(entity) {
        None => SketchError::UnknownEntity(entity),
        Some(e) => SketchError::WrongGeometry {
            entity,
            found: e.geometry.kind(),
            expected: "a line, arc or circle",
        },
    })?;
    if !(click.x.is_finite() && click.y.is_finite()) {
        return Err(SketchError::BadValue {
            kind: "trim position",
            value: if click.x.is_finite() {
                click.y
            } else {
                click.x
            },
            expected: "a finite number",
        });
    }
    let unit = scale(target).max(NEAR);
    // Every place another curve crosses this one.
    let mut cuts: Vec<Cut> = Vec::new();
    let others: Vec<EntityId> = sketch
        .entities()
        .filter(|(id, e)| *id != entity && !e.construction)
        .map(|(id, _)| id)
        .collect();
    for other in others {
        let Some(shape) = curve(sketch, other) else {
            continue;
        };
        for place in meet(target, shape) {
            let at = parameter(target, place);
            let existing = sketch
                .entities()
                .find(|(_, e)| matches!(e.geometry, Geometry::Point { at } if at.distance(place) <= NEAR))
                .map(|(id, _)| id);
            cuts.push(Cut {
                at,
                place,
                point: existing,
                cutter: other,
            });
        }
    }
    // The full extent of the parameter, and which cuts are inside it.
    let (low, high, closed) = match target {
        Curve::Line { .. } => (0.0, 1.0, false),
        Curve::Arc { sweep, .. } => (0.0, sweep, false),
        Curve::Circle { .. } => (0.0, TAU, true),
    };
    let tolerance = NEAR / unit;
    // Cuts at the very ends of a line or arc cut nothing off.
    if !closed {
        cuts.retain(|c| c.at > low + tolerance && c.at < high - tolerance);
    }
    cuts.sort_by(|a, b| a.at.total_cmp(&b.at));
    cuts.dedup_by(|a, b| (a.at - b.at).abs() <= tolerance);
    let needed = if closed { 2 } else { 1 };
    if cuts.len() < needed {
        return Err(error(format!(
            "nothing crosses {entity} {}; there is nothing to trim it against",
            if closed { "twice" } else { "between its ends" },
        )));
    }
    let clicked = parameter(target, click).clamp(low, if closed { TAU } else { high });
    // The stretch to remove runs from the cut before the click to the cut after it.
    let after = cuts.iter().position(|c| c.at > clicked);
    let (before_cut, after_cut): (Option<Cut>, Option<Cut>) = if closed {
        let first = cuts.first().copied();
        let last = cuts.last().copied();
        match after {
            Some(0) | None => (last, first),
            Some(i) => (cuts.get(i - 1).copied(), cuts.get(i).copied()),
        }
    } else {
        match after {
            None => (cuts.last().copied(), None),
            Some(0) => (None, cuts.first().copied()),
            Some(i) => (cuts.get(i - 1).copied(), cuts.get(i).copied()),
        }
    };

    // Build what is left, then remove the original.
    let mut made: Vec<(EntityId, EntityId)> = Vec::new(); // (new point, cutter)
    let mut new_point = |sketch: &mut Sketch, cut: &Cut| -> Result<EntityId, SketchError> {
        if let Some(existing) = cut.point {
            return Ok(existing);
        }
        let id = sketch.add_point(cut.place)?;
        made.push((id, cut.cutter));
        Ok(id)
    };
    let mut pieces: Vec<EntityId> = Vec::new();
    match (&target, entity_geometry(sketch, entity)?) {
        (Curve::Line { .. }, Geometry::Line { start, end }) => {
            if let Some(cut) = &before_cut {
                let p = new_point(sketch, cut)?;
                pieces.push(sketch.add_line(start, p)?);
            }
            if let Some(cut) = &after_cut {
                let p = new_point(sketch, cut)?;
                pieces.push(sketch.add_line(p, end)?);
            }
        }
        (Curve::Arc { .. }, Geometry::Arc { center, start, end }) => {
            if let Some(cut) = &before_cut {
                let p = new_point(sketch, cut)?;
                pieces.push(sketch.add_arc(center, start, p)?);
            }
            if let Some(cut) = &after_cut {
                let p = new_point(sketch, cut)?;
                pieces.push(sketch.add_arc(center, p, end)?);
            }
        }
        (Curve::Circle { .. }, Geometry::Circle { center, .. }) => {
            // What is left runs from the cut after the click round to the one before.
            if let (Some(from), Some(to)) = (&after_cut, &before_cut) {
                let start = new_point(sketch, from)?;
                let end = new_point(sketch, to)?;
                pieces.push(sketch.add_arc(center, start, end)?);
            }
        }
        _ => return Err(error("the entity changed kind while trimming".to_string())),
    }
    let construction = sketch.entity(entity).is_some_and(|e| e.construction);
    for piece in &pieces {
        sketch.set_construction(*piece, construction)?;
    }
    remove_entity_and_orphans(sketch, entity)?;
    // Hold each new end point on the curve that cut it. A refusal (the
    // constraint repeats what is already implied) is not a failure: the
    // point is already on the curve.
    for (point, cutter) in made {
        let constraint = match sketch.entity(cutter).map(|e| &e.geometry) {
            Some(Geometry::Line { .. }) => Constraint::PointOnLine {
                point,
                line: cutter,
            },
            _ => Constraint::PointOnCurve {
                point,
                curve: cutter,
            },
        };
        let _ = sketch.add_constraint(constraint, options);
    }
    Ok(())
}

fn entity_geometry(sketch: &Sketch, id: EntityId) -> Result<Geometry, SketchError> {
    sketch
        .entity(id)
        .map(|e| e.geometry.clone())
        .ok_or(SketchError::UnknownEntity(id))
}

/// Removes the entity and the points it leaves unused and unconstrained.
fn remove_entity_and_orphans(sketch: &mut Sketch, id: EntityId) -> Result<(), SketchError> {
    let points: Vec<EntityId> = match entity_geometry(sketch, id)? {
        Geometry::Line { start, end } => vec![start, end],
        Geometry::Circle { center, .. } => vec![center],
        Geometry::Arc { center, start, end } => vec![center, start, end],
        Geometry::Point { .. } => Vec::new(),
    };
    sketch.remove_entity(id)?;
    for p in points {
        let used = sketch.entities().any(|(_, e)| match e.geometry {
            Geometry::Line { start, end } => start == p || end == p,
            Geometry::Circle { center, .. } => center == p,
            Geometry::Arc { center, start, end } => center == p || start == p || end == p,
            Geometry::Point { .. } => false,
        });
        let constrained = sketch.constraints().any(|(_, c)| c.entities().contains(&p));
        if !used && !constrained && sketch.entity(p).is_some() {
            sketch.remove_entity(p)?;
        }
    }
    Ok(())
}
