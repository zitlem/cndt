// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sketch mirror and sketch patterns: copies of lines, arcs, circles and
//! points, held to the originals by symmetric and equal constraints.
//!
//! Written from the textbook reflection, translation and rotation formulas;
//! the methods are listed in `docs/provenance.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

use cndt_model::sketch::{Constraint, EntityId, Geometry, Point2, SolveOptions};
use cndt_model::{Sketch, SketchError};

/// Closer than this (millimetres) counts as the same place.
const NEAR: f64 = 1e-7;

/// Most copies a pattern may make, counting the original.
const MOST_COPIES: u32 = 200;

fn error(what: &str) -> SketchError {
    SketchError::Corrupt(what.to_owned())
}

/// The entities to copy, checked, with every point they stand on.
struct Source {
    picked: Vec<EntityId>,
    points: BTreeSet<EntityId>,
}

fn read_source(sketch: &Sketch, entities: &[EntityId]) -> Result<Source, SketchError> {
    if entities.is_empty() {
        return Err(error("pick at least one entity to copy"));
    }
    let wanted: BTreeSet<EntityId> = entities.iter().copied().collect();
    if wanted.len() != entities.len() {
        return Err(error("an entity is picked twice"));
    }
    let mut points = BTreeSet::new();
    for &id in &wanted {
        let entity = sketch.entity(id).ok_or(SketchError::UnknownEntity(id))?;
        match entity.geometry {
            Geometry::Point { .. } => {
                points.insert(id);
            }
            Geometry::Line { start, end } => points.extend([start, end]),
            Geometry::Circle { center, .. } => {
                points.insert(center);
            }
            Geometry::Arc { center, start, end } => points.extend([center, start, end]),
        }
    }
    Ok(Source {
        picked: wanted.into_iter().collect(),
        points,
    })
}

/// Old point to new point, and old entity to new entity.
type Copies = (BTreeMap<EntityId, EntityId>, BTreeMap<EntityId, EntityId>);

/// Copies the source through `place`, adding the copies to `sketch`.
///
/// `keep` says which points stay where they are (a point on a mirror line);
/// those are reused. `flip` is true for a reflection, which turns every arc
/// round.
fn copy(
    sketch: &mut Sketch,
    source: &Source,
    place: &dyn Fn(Point2) -> Point2,
    keep: &dyn Fn(Point2) -> bool,
    flip: bool,
) -> Result<Copies, SketchError> {
    let mut points: BTreeMap<EntityId, EntityId> = BTreeMap::new();
    for &id in &source.points {
        let at = sketch.point(id).ok_or(SketchError::UnknownEntity(id))?;
        if keep(at) {
            points.insert(id, id);
        } else {
            points.insert(id, sketch.add_point(place(at))?);
        }
    }
    let mut made: BTreeMap<EntityId, EntityId> = BTreeMap::new();
    for &id in &source.picked {
        let entity = sketch.entity(id).ok_or(SketchError::UnknownEntity(id))?;
        let (geometry, construction) = (entity.geometry.clone(), entity.construction);
        let at = |p: EntityId| {
            points
                .get(&p)
                .copied()
                .ok_or_else(|| error("a copy lost a point"))
        };
        let copy = match geometry {
            Geometry::Point { .. } => {
                made.insert(id, at(id)?);
                continue;
            }
            Geometry::Line { start, end } => sketch.add_line(at(start)?, at(end)?)?,
            Geometry::Circle { center, radius } => sketch.add_circle(at(center)?, radius)?,
            Geometry::Arc { center, start, end } => {
                let (center, start, end) = (at(center)?, at(start)?, at(end)?);
                if flip {
                    sketch.add_arc(center, end, start)?
                } else {
                    sketch.add_arc(center, start, end)?
                }
            }
        };
        sketch.set_construction(copy, construction)?;
        made.insert(id, copy);
    }
    Ok((points, made))
}

/// Holds each copied curve equal to its original: a line in length, a
/// circle or arc in radius.
fn hold_equal(
    sketch: &mut Sketch,
    made: &BTreeMap<EntityId, EntityId>,
    options: &SolveOptions,
) -> Result<(), SketchError> {
    for (&old, &new) in made {
        if old == new {
            continue;
        }
        let kind = sketch.entity(old).map(|e| e.geometry.kind());
        if kind.is_some_and(|k| k != "point") {
            sketch.add_constraint(Constraint::Equal { a: old, b: new }, options)?;
        }
    }
    Ok(())
}

/// Copies lines, arcs, circles and points to the other side of a line.
///
/// The copies are held by a symmetric constraint on every pair of points
/// (a circle's centre included) and, for a circle, an equal radius. Points
/// that lie on the mirror line are shared, not copied. The originals stay.
/// On a refusal the sketch is left as it was.
pub(crate) fn mirror(
    sketch: &mut Sketch,
    entities: &[EntityId],
    axis: EntityId,
) -> Result<(), SketchError> {
    let options = SolveOptions::default();
    let (a, b) = match sketch.entity(axis).map(|e| &e.geometry) {
        Some(&Geometry::Line { start, end }) => (
            sketch
                .point(start)
                .ok_or(SketchError::UnknownEntity(start))?,
            sketch.point(end).ok_or(SketchError::UnknownEntity(end))?,
        ),
        Some(other) => {
            return Err(SketchError::WrongGeometry {
                entity: axis,
                found: other.kind(),
                expected: "a line",
            });
        }
        None => return Err(SketchError::UnknownEntity(axis)),
    };
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy);
    if length <= NEAR {
        return Err(error("the mirror line has no length"));
    }
    if entities.contains(&axis) {
        return Err(error(
            "the mirror line cannot be mirrored with its own side",
        ));
    }
    let source = read_source(sketch, entities)?;
    // Signed distance from the line, and the reflection through it.
    let side = |p: Point2| (dx * (p.y - a.y) - dy * (p.x - a.x)) / length;
    let reflect = |p: Point2| {
        let d = side(p);
        Point2::new(p.x + 2.0 * d * dy / length, p.y - 2.0 * d * dx / length)
    };
    for &id in &source.picked {
        let Some(entity) = sketch.entity(id) else {
            continue;
        };
        let on_line = |p: EntityId| sketch.point(p).is_some_and(|at| side(at).abs() <= NEAR);
        let sits_on_axis = match entity.geometry {
            Geometry::Line { start, end } => on_line(start) && on_line(end),
            Geometry::Circle { center, .. } => on_line(center),
            _ => false,
        };
        if sits_on_axis {
            return Err(error(&format!(
                "{id} lies on the mirror line, so its mirror image would sit on top of it"
            )));
        }
    }
    let mut work = sketch.clone();
    let (points, made) = copy(
        &mut work,
        &source,
        &reflect,
        &|p| side(p).abs() <= NEAR,
        true,
    )?;
    for (&old, &new) in &points {
        if old != new {
            let held = Constraint::Symmetric {
                a: old,
                b: new,
                axis,
            };
            match work.add_constraint(held, &options) {
                Ok(_) => {}
                // An arc keeps its ends on one circle, so once its centre and
                // one end are held, a whole symmetric constraint on the
                // other end would repeat one equation already implied. That
                // end starts mirrored but is free along the circle.
                Err(SketchError::Redundant { .. }) => {}
                Err(other) => return Err(other),
            }
        }
    }
    // Arcs are held by their three points and lines by their two; only a
    // circle has a size of its own.
    for (&old, &new) in &made {
        if work.entity(old).map(|e| e.geometry.kind()) == Some("circle") {
            work.add_constraint(Constraint::Equal { a: old, b: new }, &options)?;
        }
    }
    *sketch = work;
    Ok(())
}

fn check_count(count: u32) -> Result<(), SketchError> {
    if (2..=MOST_COPIES).contains(&count) {
        Ok(())
    } else {
        Err(SketchError::BadValue {
            kind: "pattern count",
            value: f64::from(count),
            expected: "from 2 to 200, the original included",
        })
    }
}

/// Repeats the picked entities `count` times in all (the original and
/// `count - 1` copies), each `step` further along than the last. Every copy
/// is held equal to its original in length or radius.
pub(crate) fn pattern_linear(
    sketch: &mut Sketch,
    entities: &[EntityId],
    count: u32,
    step: Point2,
) -> Result<(), SketchError> {
    check_count(count)?;
    if !(step.x.is_finite() && step.y.is_finite()) || step.x.hypot(step.y) <= NEAR {
        return Err(error("the step between copies must be more than zero"));
    }
    let source = read_source(sketch, entities)?;
    let options = SolveOptions::default();
    let mut work = sketch.clone();
    for n in 1..count {
        let k = f64::from(n);
        let (mx, my) = (step.x * k, step.y * k);
        let (_, made) = copy(
            &mut work,
            &source,
            &|p| Point2::new(p.x + mx, p.y + my),
            &|_| false,
            false,
        )?;
        hold_equal(&mut work, &made, &options)?;
    }
    *sketch = work;
    Ok(())
}

/// Repeats the picked entities `count` times in all, turned about `center`.
/// A full turn (360) spaces them `360 / count` apart; a smaller `angle` is
/// the turn from the original to the last copy. Every copy is held equal to
/// its original in length or radius.
pub(crate) fn pattern_circular(
    sketch: &mut Sketch,
    entities: &[EntityId],
    count: u32,
    center: Point2,
    angle: f64,
) -> Result<(), SketchError> {
    check_count(count)?;
    if !(angle.is_finite() && angle > 0.0 && angle <= 360.0) {
        return Err(SketchError::BadValue {
            kind: "pattern angle",
            value: angle,
            expected: "more than 0 and at most 360 degrees",
        });
    }
    if !(center.x.is_finite() && center.y.is_finite()) {
        return Err(error("the centre of the pattern must be a finite position"));
    }
    let source = read_source(sketch, entities)?;
    let options = SolveOptions::default();
    let full = (angle - 360.0).abs() < 1e-9;
    let spacing = if full {
        TAU / f64::from(count)
    } else {
        angle.to_radians() / f64::from(count - 1)
    };
    let mut work = sketch.clone();
    for n in 1..count {
        let (sin, cos) = (spacing * f64::from(n)).sin_cos();
        let (_, made) = copy(
            &mut work,
            &source,
            &|p| {
                let (x, y) = (p.x - center.x, p.y - center.y);
                Point2::new(center.x + x * cos - y * sin, center.y + x * sin + y * cos)
            },
            &|_| false,
            false,
        )?;
        hold_equal(&mut work, &made, &options)?;
    }
    *sketch = work;
    Ok(())
}

#[cfg(test)]
mod tests;
