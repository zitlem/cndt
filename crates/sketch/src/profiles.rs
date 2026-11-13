// SPDX-License-Identifier: AGPL-3.0-or-later
//! Profiles: the closed regions of a sketch that can be extruded.
//!
//! A loop is a closed chain of lines and arcs that meet end to end, or a
//! single circle. Construction geometry is ignored. As in Fusion 360, every
//! loop bounds a region, and the loops directly inside it are that region's
//! holes: a plate outline with a circle in it gives the plate with a hole,
//! and the disc inside the circle.
//!
//! A profile is named by the entities of its outer boundary, so the name
//! survives dimension changes (decision 0012).
//!
//! Not handled yet: chains that branch (three curves meeting at one point)
//! and loops that cross each other. Their curves are reported as not closed,
//! never silently dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{EntityId, Geometry, Point2, Sketch};

/// Names a profile: the sorted ids of its outer boundary's curves.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileKey(pub Vec<EntityId>);

/// One curve of a loop, in the loop's running order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopCurve {
    /// The line, arc or circle.
    pub entity: EntityId,
    /// True when the loop runs from the curve's end to its start.
    pub reversed: bool,
}

/// A closed chain of curves.
#[derive(Debug, Clone, PartialEq)]
pub struct Loop {
    /// The curves in running order.
    pub curves: Vec<LoopCurve>,
    /// Enclosed area in square millimetres.
    pub area: f64,
}

/// A region that can be extruded: an outer boundary and its holes.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Its stable name.
    pub key: ProfileKey,
    /// The outer boundary.
    pub outer: Loop,
    /// Loops directly inside it.
    pub holes: Vec<Loop>,
    /// How many loops enclose it. Even depths are material when a sketch is
    /// read as a part outline (a plate); odd depths are the insides of holes.
    pub depth: usize,
}

/// Every profile in a sketch, and the curves that are not part of one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profiles {
    /// The regions, largest first.
    pub profiles: Vec<Profile>,
    /// Non-construction curves in no closed loop, in id order.
    pub open: Vec<EntityId>,
}

impl Profiles {
    /// The profile with this key.
    #[must_use]
    pub fn get(&self, key: &ProfileKey) -> Option<&Profile> {
        self.profiles.iter().find(|p| &p.key == key)
    }
}

/// Sample points along a curve, in running order, excluding its last point.
fn samples(sketch: &Sketch, curve: LoopCurve) -> Vec<Point2> {
    let at = |id: EntityId| sketch.point(id);
    let mut out = Vec::new();
    match sketch.entity(curve.entity).map(|e| &e.geometry) {
        Some(Geometry::Line { start, end }) => {
            let (a, b) = if curve.reversed {
                (*end, *start)
            } else {
                (*start, *end)
            };
            if let Some(p) = at(a) {
                out.push(p);
            }
            let _ = b;
        }
        Some(Geometry::Arc { center, start, end }) => {
            if let (Some(c), Some(s), Some(e)) = (at(*center), at(*start), at(*end)) {
                let r = c.distance(s);
                let from = (s.y - c.y).atan2(s.x - c.x);
                let to = (e.y - c.y).atan2(e.x - c.x);
                let sweep = (to - from).rem_euclid(std::f64::consts::TAU);
                let steps = 32;
                for k in 0..steps {
                    let t = if curve.reversed { steps - k } else { k };
                    let angle = from + sweep * f64::from(t) / f64::from(steps);
                    out.push(Point2::new(c.x + r * angle.cos(), c.y + r * angle.sin()));
                }
            }
        }
        Some(Geometry::Circle { center, radius }) => {
            if let Some(c) = at(*center) {
                for k in 0..64 {
                    let angle = std::f64::consts::TAU * f64::from(k) / 64.0;
                    out.push(Point2::new(
                        c.x + radius * angle.cos(),
                        c.y + radius * angle.sin(),
                    ));
                }
            }
        }
        _ => {}
    }
    out
}

fn polygon(sketch: &Sketch, curves: &[LoopCurve]) -> Vec<Point2> {
    curves.iter().flat_map(|c| samples(sketch, *c)).collect()
}

fn signed_area(polygon: &[Point2]) -> f64 {
    let n = polygon.len();
    (0..n)
        .filter_map(|i| Some((polygon.get(i)?, polygon.get((i + 1) % n)?)))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        / 2.0
}

fn inside(p: Point2, polygon: &[Point2]) -> bool {
    let n = polygon.len();
    let mut crossings = false;
    for i in 0..n {
        let (Some(a), Some(b)) = (polygon.get(i), polygon.get((i + 1) % n)) else {
            continue;
        };
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                crossings = !crossings;
            }
        }
    }
    crossings
}

/// The end points of a line or arc.
fn ends(geometry: &Geometry) -> Option<(EntityId, EntityId)> {
    match *geometry {
        Geometry::Line { start, end } | Geometry::Arc { start, end, .. } => Some((start, end)),
        _ => None,
    }
}

impl Sketch {
    /// Finds the closed regions of the sketch.
    #[must_use]
    pub fn profiles(&self) -> Profiles {
        let mut loops: Vec<Vec<LoopCurve>> = Vec::new();
        let mut open = Vec::new();
        // Point → the chain curves that end there.
        let mut at_point: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for (id, entity) in self.entities() {
            if entity.construction {
                continue;
            }
            match &entity.geometry {
                Geometry::Circle { .. } => loops.push(vec![LoopCurve {
                    entity: id,
                    reversed: false,
                }]),
                g => {
                    if let Some((a, b)) = ends(g) {
                        at_point.entry(a).or_default().push(id);
                        at_point.entry(b).or_default().push(id);
                    }
                }
            }
        }
        let chain_curves: BTreeSet<EntityId> = at_point.values().flatten().copied().collect();
        let mut seen = BTreeSet::new();
        for &first in &chain_curves {
            if seen.contains(&first) {
                continue;
            }
            // Collect the connected group of curves.
            let mut group = BTreeSet::from([first]);
            let mut stack = vec![first];
            while let Some(curve) = stack.pop() {
                let Some((a, b)) = self.entity(curve).and_then(|e| ends(&e.geometry)) else {
                    continue;
                };
                for p in [a, b] {
                    for &next in at_point.get(&p).into_iter().flatten() {
                        if group.insert(next) {
                            stack.push(next);
                        }
                    }
                }
            }
            seen.extend(group.iter().copied());
            // A simple loop: every point in the group joins exactly two curves.
            let points: BTreeSet<EntityId> = group
                .iter()
                .filter_map(|c| self.entity(*c).and_then(|e| ends(&e.geometry)))
                .flat_map(|(a, b)| [a, b])
                .collect();
            let simple = points
                .iter()
                .all(|p| at_point.get(p).is_some_and(|c| c.len() == 2))
                && points.len() == group.len();
            if !simple {
                open.extend(group.iter().copied());
                continue;
            }
            // Walk it from the lowest curve id, forwards.
            let mut walk = Vec::with_capacity(group.len());
            let mut curve = first;
            let Some((start, mut end)) = self.entity(first).and_then(|e| ends(&e.geometry)) else {
                continue;
            };
            walk.push(LoopCurve {
                entity: first,
                reversed: false,
            });
            while end != start && walk.len() <= group.len() {
                let Some(&next) = at_point
                    .get(&end)
                    .and_then(|c| c.iter().find(|&&n| n != curve))
                else {
                    break;
                };
                let Some((a, b)) = self.entity(next).and_then(|e| ends(&e.geometry)) else {
                    break;
                };
                let reversed = b == end;
                end = if reversed { a } else { b };
                walk.push(LoopCurve {
                    entity: next,
                    reversed,
                });
                curve = next;
            }
            if end == start && walk.len() == group.len() {
                loops.push(walk);
            } else {
                open.extend(group.iter().copied());
            }
        }
        open.sort_unstable();

        // Nesting.
        let shapes: Vec<(Vec<LoopCurve>, Vec<Point2>, f64)> = loops
            .into_iter()
            .map(|curves| {
                let poly = polygon(self, &curves);
                let area = signed_area(&poly).abs();
                (curves, poly, area)
            })
            .collect();
        let parent: Vec<Option<usize>> = shapes
            .iter()
            .enumerate()
            .map(|(i, (_, poly, area))| {
                let probe = poly.first().copied()?;
                shapes
                    .iter()
                    .enumerate()
                    .filter(|(j, (_, other, other_area))| {
                        *j != i && other_area > area && inside(probe, other)
                    })
                    .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
                    .map(|(j, _)| j)
            })
            .collect();
        let depth = |mut i: usize| {
            let mut d = 0;
            while let Some(Some(p)) = parent.get(i) {
                d += 1;
                i = *p;
                if d > shapes.len() {
                    break;
                }
            }
            d
        };
        let as_loop = |i: usize| {
            shapes.get(i).map(|(curves, _, area)| Loop {
                curves: curves.clone(),
                area: *area,
            })
        };
        let mut profiles: Vec<Profile> = Vec::new();
        for i in 0..shapes.len() {
            let Some(outer) = as_loop(i) else { continue };
            let holes = (0..shapes.len())
                .filter(|j| parent.get(*j).copied().flatten() == Some(i))
                .filter_map(as_loop)
                .collect();
            let mut key: Vec<EntityId> = outer.curves.iter().map(|c| c.entity).collect();
            key.sort_unstable();
            profiles.push(Profile {
                key: ProfileKey(key),
                outer,
                holes,
                depth: depth(i),
            });
        }
        profiles.sort_by(|a, b| {
            b.outer
                .area
                .total_cmp(&a.outer.area)
                .then_with(|| a.key.cmp(&b.key))
        });
        Profiles { profiles, open }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    fn pt(s: &mut Sketch, x: f64, y: f64) -> EntityId {
        s.add_point(Point2::new(x, y)).unwrap()
    }

    fn rectangle(s: &mut Sketch, x: f64, y: f64, w: f64, h: f64) -> Vec<EntityId> {
        let p = [
            pt(s, x, y),
            pt(s, x + w, y),
            pt(s, x + w, y + h),
            pt(s, x, y + h),
        ];
        (0..4)
            .map(|i| s.add_line(p[i], p[(i + 1) % 4]).unwrap())
            .collect()
    }

    #[test]
    fn a_plate_with_a_pocket_outline_and_holes() {
        let mut s = Sketch::new();
        let outer = rectangle(&mut s, 0.0, 0.0, 96.0, 56.0);
        let pocket = rectangle(&mut s, 28.0, 18.0, 40.0, 20.0);
        let c = pt(&mut s, 10.0, 10.0);
        let hole = s.add_circle(c, 3.3).unwrap();
        let found = s.profiles();
        assert!(found.open.is_empty());
        assert_eq!(
            found.profiles.len(),
            3,
            "the plate, the pocket area, the disc in the hole"
        );
        let plate = &found.profiles[0];
        assert_eq!(plate.key.0, outer);
        assert!((plate.outer.area - 96.0 * 56.0).abs() < 1e-9);
        assert_eq!(plate.holes.len(), 2, "the pocket outline and the hole");
        let inner = found.get(&ProfileKey(pocket.clone())).unwrap();
        assert!(inner.holes.is_empty());
        assert!(plate.holes.iter().any(|h| h.curves[0].entity == hole));
    }

    #[test]
    fn a_slot_with_arcs_runs_end_to_end() {
        let mut s = Sketch::new();
        let (lc, rc) = (pt(&mut s, 0.0, 0.0), pt(&mut s, 22.0, 0.0));
        let (r_lo, r_hi) = (pt(&mut s, 22.0, -4.0), pt(&mut s, 22.0, 4.0));
        let (l_hi, l_lo) = (pt(&mut s, 0.0, 4.0), pt(&mut s, 0.0, -4.0));
        let right = s.add_arc(rc, r_lo, r_hi).unwrap();
        let top = s.add_line(r_hi, l_hi).unwrap();
        let left = s.add_arc(lc, l_hi, l_lo).unwrap();
        // Drawn the other way round, to check reversal.
        let bottom = s.add_line(r_lo, l_lo).unwrap();
        let found = s.profiles();
        assert_eq!(found.profiles.len(), 1);
        let profile = &found.profiles[0];
        let order: Vec<(EntityId, bool)> = profile
            .outer
            .curves
            .iter()
            .map(|c| (c.entity, c.reversed))
            .collect();
        assert_eq!(
            order,
            [(right, false), (top, false), (left, false), (bottom, true)]
        );
        let expected = 22.0 * 8.0 + std::f64::consts::PI * 16.0;
        assert!(
            (profile.outer.area - expected).abs() < 0.2,
            "{}",
            profile.outer.area
        );
    }

    #[test]
    fn open_and_construction_geometry_is_not_a_profile() {
        let mut s = Sketch::new();
        let (a, b, c) = (
            pt(&mut s, 0.0, 0.0),
            pt(&mut s, 10.0, 0.0),
            pt(&mut s, 10.0, 10.0),
        );
        let l1 = s.add_line(a, b).unwrap();
        let l2 = s.add_line(b, c).unwrap();
        let centre = pt(&mut s, 50.0, 50.0);
        let guide = s.add_circle(centre, 5.0).unwrap();
        s.set_construction(guide, true).unwrap();
        let found = s.profiles();
        assert!(found.profiles.is_empty());
        assert_eq!(found.open, [l1, l2]);
        // A branch (three lines meeting) is not closed either.
        let mut s = Sketch::new();
        let lines = rectangle(&mut s, 0.0, 0.0, 10.0, 10.0);
        let d = pt(&mut s, 20.0, 0.0);
        let Some(Geometry::Line { end, .. }) = s.entity(lines[0]).map(|e| e.geometry.clone())
        else {
            panic!()
        };
        s.add_line(end, d).unwrap();
        assert!(s.profiles().profiles.is_empty());
    }

    #[test]
    fn keys_survive_dimension_changes() {
        let mut s = Sketch::new();
        let lines = rectangle(&mut s, 0.0, 0.0, 10.0, 10.0);
        let before = s.profiles().profiles[0].key.clone();
        let opts = crate::SolveOptions::default();
        let len = s
            .add_constraint(
                crate::Constraint::Length {
                    line: lines[0],
                    value: 10.0,
                },
                &opts,
            )
            .unwrap();
        s.set_dimension(len, 25.0, &opts).unwrap();
        assert_eq!(s.profiles().profiles[0].key, before);
    }
}
