// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offset: copy a chain of lines and arcs, or a circle, a set distance to one
//! side of the original.
//!
//! Written from the textbook formulas for parallel lines and concentric
//! circles; the method is listed in `docs/provenance.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

use cndt_model::sketch::{EntityId, Geometry, Point2};
use cndt_model::{Sketch, SketchError};

/// Closer than this (millimetres) counts as the same place.
const NEAR: f64 = 1e-7;

fn error(what: &str) -> SketchError {
    SketchError::Corrupt(what.to_owned())
}

/// One piece of the chain, in the direction the chain is walked.
#[derive(Debug, Clone, Copy)]
struct Piece {
    /// The sketch entity it was read from.
    entity: EntityId,
    from: Point2,
    to: Point2,
    arc: Option<Arc>,
}

#[derive(Debug, Clone, Copy)]
struct Arc {
    center_id: EntityId,
    center: Point2,
    radius: f64,
    /// Whether the walk runs counter-clockwise round the centre.
    ccw: bool,
}

/// The unit direction a piece runs in at `at`, one of its points.
fn tangent(piece: &Piece, at: Point2) -> (f64, f64) {
    if let Some(arc) = piece.arc {
        let (rx, ry) = (
            (at.x - arc.center.x) / arc.radius,
            (at.y - arc.center.y) / arc.radius,
        );
        if arc.ccw { (-ry, rx) } else { (ry, -rx) }
    } else {
        let (dx, dy) = (piece.to.x - piece.from.x, piece.to.y - piece.from.y);
        let length = dx.hypot(dy);
        (dx / length, dy / length)
    }
}

/// A moved piece's whole curve, for finding where neighbours meet.
#[derive(Debug, Clone, Copy)]
enum Whole {
    Line { a: Point2, b: Point2 },
    Circle { center: Point2, radius: f64 },
}

fn meets(first: Whole, second: Whole) -> Vec<Point2> {
    match (first, second) {
        (Whole::Line { a: p, b: q }, Whole::Line { a: r, b: s }) => {
            let (rx, ry) = (q.x - p.x, q.y - p.y);
            let (sx, sy) = (s.x - r.x, s.y - r.y);
            let denominator = rx * sy - ry * sx;
            if denominator.abs() <= 1e-12 {
                return Vec::new();
            }
            let t = ((r.x - p.x) * sy - (r.y - p.y) * sx) / denominator;
            vec![Point2::new(p.x + t * rx, p.y + t * ry)]
        }
        (Whole::Line { a, b }, Whole::Circle { center, radius })
        | (Whole::Circle { center, radius }, Whole::Line { a, b }) => {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let (fx, fy) = (a.x - center.x, a.y - center.y);
            let qa = dx * dx + dy * dy;
            let qb = 2.0 * (fx * dx + fy * dy);
            let qc = fx * fx + fy * fy - radius * radius;
            let disc = qb * qb - 4.0 * qa * qc;
            if qa <= 0.0 || disc < 0.0 {
                return Vec::new();
            }
            let root = disc.sqrt();
            [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)]
                .into_iter()
                .map(|t| Point2::new(a.x + t * dx, a.y + t * dy))
                .collect()
        }
        (
            Whole::Circle {
                center: c1,
                radius: r1,
            },
            Whole::Circle {
                center: c2,
                radius: r2,
            },
        ) => crate::sketch_trim::circle_circle(c1, r1, c2, r2),
    }
}

/// What to make, worked out before the sketch is touched.
enum Made {
    Circle {
        center: Point2,
        radius: f64,
        construction: bool,
    },
    Chain {
        points: Vec<Point2>,
        pieces: Vec<Piece>,
        closed: bool,
    },
}

/// Offsets `entities` by `distance` (more than zero) toward the side of them
/// that holds `toward`.
///
/// `entities` are one circle, or lines and arcs that join end to end into a
/// single chain, open or closed. The copies are new, unconstrained
/// geometry; the originals stay. Corners are extended to meet (a mitre),
/// not rounded. A copy that would collapse (a line that reverses, an arc
/// that loses its radius) is refused and the sketch is left as it was.
pub(crate) fn offset(
    sketch: &mut Sketch,
    entities: &[EntityId],
    distance: f64,
    toward: Point2,
) -> Result<(), SketchError> {
    if !(distance.is_finite() && distance > NEAR && distance <= 1e6) {
        return Err(SketchError::BadValue {
            kind: "offset distance",
            value: distance,
            expected: "more than zero and at most 1000000",
        });
    }
    for value in [toward.x, toward.y] {
        if !value.is_finite() {
            return Err(SketchError::BadValue {
                kind: "offset side",
                value,
                expected: "a finite number",
            });
        }
    }
    if entities.is_empty() {
        return Err(error("pick at least one line, arc or circle to offset"));
    }
    let wanted: BTreeSet<EntityId> = entities.iter().copied().collect();
    if wanted.len() != entities.len() {
        return Err(error("an entity is picked twice"));
    }
    match plan(sketch, &wanted, distance, toward)? {
        Made::Circle {
            center,
            radius,
            construction,
        } => {
            let c = sketch.add_point(center)?;
            let circle = sketch.add_circle(c, radius)?;
            sketch.set_construction(circle, construction)?;
        }
        Made::Chain {
            points,
            pieces,
            closed,
        } => {
            let ids: Vec<EntityId> = points
                .iter()
                .map(|p| sketch.add_point(*p))
                .collect::<Result<_, _>>()?;
            let mut centers: BTreeMap<EntityId, EntityId> = BTreeMap::new();
            for (i, piece) in pieces.iter().enumerate() {
                let next = if closed && i + 1 == pieces.len() {
                    0
                } else {
                    i + 1
                };
                let (Some(&start), Some(&end)) = (ids.get(i), ids.get(next)) else {
                    return Err(error("offset lost a point"));
                };
                let made = if let Some(arc) = piece.arc {
                    let center = if let Some(c) = centers.get(&arc.center_id) {
                        *c
                    } else {
                        let c = sketch.add_point(arc.center)?;
                        centers.insert(arc.center_id, c);
                        c
                    };
                    if arc.ccw {
                        sketch.add_arc(center, start, end)?
                    } else {
                        sketch.add_arc(center, end, start)?
                    }
                } else {
                    sketch.add_line(start, end)?
                };
                let construction = sketch.entity(piece.entity).is_some_and(|e| e.construction);
                sketch.set_construction(made, construction)?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn plan(
    sketch: &Sketch,
    wanted: &BTreeSet<EntityId>,
    distance: f64,
    toward: Point2,
) -> Result<Made, SketchError> {
    // Read every picked entity.
    let mut segments: Vec<(EntityId, EntityId, EntityId)> = Vec::new(); // (entity, start, end)
    for &id in wanted {
        let entity = sketch.entity(id).ok_or(SketchError::UnknownEntity(id))?;
        match entity.geometry {
            Geometry::Circle { center, radius } => {
                if wanted.len() > 1 {
                    return Err(error(
                        "a circle is offset on its own, not with other curves",
                    ));
                }
                let c = sketch
                    .point(center)
                    .ok_or(SketchError::UnknownEntity(center))?;
                let gap = c.distance(toward) - radius;
                if gap.abs() <= NEAR {
                    return Err(error("the side to offset toward lies on the circle"));
                }
                let new_radius = if gap < 0.0 {
                    radius - distance
                } else {
                    radius + distance
                };
                if new_radius <= NEAR {
                    return Err(error(&format!(
                        "offsetting {id} inward by {distance} mm leaves no circle: its radius is {radius} mm"
                    )));
                }
                return Ok(Made::Circle {
                    center: c,
                    radius: new_radius,
                    construction: entity.construction,
                });
            }
            Geometry::Line { start, end } | Geometry::Arc { start, end, .. } => {
                segments.push((id, start, end));
            }
            Geometry::Point { .. } | Geometry::Spline { .. } => {
                return Err(SketchError::WrongGeometry {
                    entity: id,
                    found: entity.geometry.kind(),
                    expected: "a line, arc or circle",
                });
            }
        }
    }
    // Which pieces meet at each point.
    let mut at_point: BTreeMap<EntityId, Vec<usize>> = BTreeMap::new();
    for (i, &(_, start, end)) in segments.iter().enumerate() {
        at_point.entry(start).or_default().push(i);
        at_point.entry(end).or_default().push(i);
    }
    if at_point.values().any(|v| v.len() > 2) {
        return Err(error("the picked curves branch; offset takes one chain"));
    }
    let ends: Vec<EntityId> = at_point
        .iter()
        .filter(|(_, v)| v.len() == 1)
        .map(|(p, _)| *p)
        .collect();
    let closed = ends.is_empty();
    if !closed && ends.len() != 2 {
        return Err(error("the picked curves are not one chain"));
    }
    // Walk the chain.
    let mut current = match ends.first() {
        Some(p) => *p,
        None => segments
            .first()
            .map(|s| s.1)
            .ok_or_else(|| error("nothing picked"))?,
    };
    let mut used = vec![false; segments.len()];
    let mut pieces: Vec<Piece> = Vec::new();
    while let Some(&i) = at_point
        .get(&current)
        .and_then(|v| v.iter().find(|&&i| !used.get(i).copied().unwrap_or(true)))
    {
        if let Some(flag) = used.get_mut(i) {
            *flag = true;
        }
        let &(entity, start, end) = segments.get(i).ok_or_else(|| error("lost a piece"))?;
        let forward = start == current;
        let (a, b) = if forward { (start, end) } else { (end, start) };
        let (pa, pb) = (
            sketch.point(a).ok_or(SketchError::UnknownEntity(a))?,
            sketch.point(b).ok_or(SketchError::UnknownEntity(b))?,
        );
        let arc = match sketch.entity(entity).map(|e| &e.geometry) {
            Some(Geometry::Arc { center, start, .. }) => {
                let c = sketch
                    .point(*center)
                    .ok_or(SketchError::UnknownEntity(*center))?;
                let s = sketch
                    .point(*start)
                    .ok_or(SketchError::UnknownEntity(*start))?;
                Some(Arc {
                    center_id: *center,
                    center: c,
                    radius: c.distance(s),
                    ccw: forward,
                })
            }
            _ => None,
        };
        pieces.push(Piece {
            entity,
            from: pa,
            to: pb,
            arc,
        });
        current = b;
    }
    if used.iter().any(|u| !u) {
        return Err(error("the picked curves are not one chain"));
    }
    // Which side: the side of the piece nearest `toward`, as left or right of the walk.
    let mut best: Option<(f64, f64)> = None; // (gap, side)
    for piece in &pieces {
        let (gap, left) = if let Some(arc) = piece.arc {
            let from_center = arc.center.distance(toward);
            (
                (from_center - arc.radius).abs(),
                if (from_center < arc.radius) == arc.ccw {
                    1.0
                } else {
                    -1.0
                },
            )
        } else {
            let (dx, dy) = (piece.to.x - piece.from.x, piece.to.y - piece.from.y);
            let length = dx.hypot(dy);
            let t = (((toward.x - piece.from.x) * dx + (toward.y - piece.from.y) * dy)
                / (length * length))
                .clamp(0.0, 1.0);
            let foot = Point2::new(piece.from.x + t * dx, piece.from.y + t * dy);
            (
                foot.distance(toward),
                dx * (toward.y - piece.from.y) - dy * (toward.x - piece.from.x),
            )
        };
        if best.is_none_or(|(g, _)| gap < g) {
            best = Some((gap, left));
        }
    }
    let (gap, left) = best.ok_or_else(|| error("nothing picked"))?;
    if gap <= NEAR || left == 0.0 {
        return Err(error("the side to offset toward lies on the curve"));
    }
    let d = if left > 0.0 { distance } else { -distance };
    // Each piece moved d to its left, as a whole curve.
    let mut wholes: Vec<Whole> = Vec::new();
    let mut new_radii: Vec<Option<f64>> = Vec::new();
    for piece in &pieces {
        if let Some(arc) = piece.arc {
            let radius = if arc.ccw {
                arc.radius - d
            } else {
                arc.radius + d
            };
            if radius <= NEAR {
                return Err(error(&format!(
                    "the arc {} has radius {} mm, so offsetting it {distance} mm to that side leaves nothing",
                    piece.entity, arc.radius
                )));
            }
            wholes.push(Whole::Circle {
                center: arc.center,
                radius,
            });
            new_radii.push(Some(radius));
        } else {
            let (tx, ty) = tangent(piece, piece.from);
            let (nx, ny) = (-ty * d, tx * d);
            wholes.push(Whole::Line {
                a: Point2::new(piece.from.x + nx, piece.from.y + ny),
                b: Point2::new(piece.to.x + nx, piece.to.y + ny),
            });
            new_radii.push(None);
        }
    }
    // The new corners: where neighbouring copies meet, nearest the old corner.
    let count = pieces.len();
    let joints = if closed { count } else { count + 1 };
    let mut points: Vec<Point2> = Vec::with_capacity(joints);
    for j in 0..joints {
        let before = if j == 0 && !closed {
            None
        } else {
            pieces.get((j + count - 1) % count)
        };
        let after = if j == count {
            None
        } else {
            pieces.get(j % count)
        };
        let (Some(first), Some(second)) = (before, after) else {
            // A free end: the old end moved straight sideways.
            let (piece, at) = match (before, after) {
                (Some(p), _) => (p, p.to),
                (None, Some(p)) => (p, p.from),
                (None, None) => return Err(error("nothing picked")),
            };
            let (tx, ty) = tangent(piece, at);
            points.push(Point2::new(at.x - ty * d, at.y + tx * d));
            continue;
        };
        let corner = first.to;
        let (t1, t2) = (tangent(first, corner), tangent(second, corner));
        let cross = t1.0 * t2.1 - t1.1 * t2.0;
        let dot = t1.0 * t2.0 + t1.1 * t2.1;
        if cross.abs() < 1e-9 && dot > 0.0 {
            // Tangent neighbours: the corner moves straight sideways.
            points.push(Point2::new(corner.x - t1.1 * d, corner.y + t1.0 * d));
            continue;
        }
        let (Some(&a), Some(&b)) = (wholes.get((j + count - 1) % count), wholes.get(j % count))
        else {
            return Err(error("offset lost a piece"));
        };
        let hit = meets(a, b)
            .into_iter()
            .min_by(|p, q| p.distance(corner).total_cmp(&q.distance(corner)))
            .ok_or_else(|| {
                error(&format!(
                    "the offset curves next to ({:.3}, {:.3}) do not meet, so the corner cannot be rebuilt",
                    corner.x, corner.y
                ))
            })?;
        points.push(hit);
    }
    // Nothing may have collapsed or turned round.
    for (i, piece) in pieces.iter().enumerate() {
        let next = if closed && i + 1 == count { 0 } else { i + 1 };
        let (Some(&start), Some(&end)) = (points.get(i), points.get(next)) else {
            return Err(error("offset lost a point"));
        };
        let collapsed = || {
            error(&format!(
                "offsetting {} by {distance} mm to that side collapses it",
                piece.entity
            ))
        };
        if let Some(arc) = piece.arc {
            let angle = |p: Point2| (p.y - arc.center.y).atan2(p.x - arc.center.x);
            let (low, high, old_from, old_to) = if arc.ccw {
                (angle(start), angle(end), angle(piece.from), angle(piece.to))
            } else {
                (angle(end), angle(start), angle(piece.to), angle(piece.from))
            };
            let old_sweep = (old_to - old_from).rem_euclid(TAU);
            let sweep = (high - low).rem_euclid(TAU);
            let middle = old_from + old_sweep / 2.0;
            let holds_middle = (middle - low).rem_euclid(TAU) <= sweep;
            if sweep <= 1e-9 || !holds_middle || sweep > old_sweep + PI {
                return Err(collapsed());
            }
        } else {
            let (ox, oy) = (piece.to.x - piece.from.x, piece.to.y - piece.from.y);
            let (nx, ny) = (end.x - start.x, end.y - start.y);
            if nx.hypot(ny) <= NEAR || ox * nx + oy * ny <= 0.0 {
                return Err(collapsed());
            }
        }
    }
    Ok(Made::Chain {
        points,
        pieces,
        closed,
    })
}

#[cfg(test)]
mod tests;
