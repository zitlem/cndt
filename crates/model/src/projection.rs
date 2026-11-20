// SPDX-License-Identifier: AGPL-3.0-or-later
//! Model edges projected into a sketch (decision 0033).
//!
//! An edge's outline, already straight-projected onto the sketch plane, is
//! fitted to a line, an arc or a circle. Anything else is refused. The fit
//! is the textbook circumcircle of three points, checked against every
//! point of the outline.

use cndt_geom::Point2;
use cndt_sketch::{EntityId, Geometry, Sketch};
use serde::{Deserialize, Serialize};

/// How far (millimetres) an outline may stray from a fitted line or circle,
/// and how far a stored projection may differ from a fresh one. The
/// kernel's outlines are 32-bit floats, good to about 1e-5 at 100 mm.
pub const TOLERANCE: f64 = 1e-3;

/// A model edge that a sketch entity was projected from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    /// The name of one of the two faces that meet at the edge.
    pub first: String,
    /// The name of the other.
    pub second: String,
    /// The line, arc or circle the projection made.
    pub entity: EntityId,
}

/// What an edge looks like in the sketch plane.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectedShape {
    /// A straight line.
    Line {
        /// One end.
        start: Point2,
        /// The other end.
        end: Point2,
    },
    /// A counter-clockwise arc.
    Arc {
        /// The centre.
        center: Point2,
        /// Where it starts.
        start: Point2,
        /// Where it ends.
        end: Point2,
    },
    /// A full circle.
    Circle {
        /// The centre.
        center: Point2,
        /// The radius.
        radius: f64,
    },
}

fn near(a: Point2, b: Point2) -> bool {
    a.distance(b) <= TOLERANCE
}

impl ProjectedShape {
    /// True if `other` is the same shape within [`TOLERANCE`].
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Line { start: a, end: b }, Self::Line { start: c, end: d }) => {
                (near(a, c) && near(b, d)) || (near(a, d) && near(b, c))
            }
            (
                Self::Arc {
                    center: a,
                    start: b,
                    end: c,
                },
                Self::Arc {
                    center: d,
                    start: e,
                    end: f,
                },
            ) => near(a, d) && near(b, e) && near(c, f),
            (
                Self::Circle {
                    center: a,
                    radius: r,
                },
                Self::Circle {
                    center: b,
                    radius: s,
                },
            ) => near(a, b) && (r - s).abs() <= TOLERANCE,
            _ => false,
        }
    }

    /// The shape of a line, arc or circle in a sketch.
    #[must_use]
    pub fn of_entity(sketch: &Sketch, entity: EntityId) -> Option<Self> {
        match sketch.entity(entity)?.geometry {
            Geometry::Line { start, end } => Some(Self::Line {
                start: sketch.point(start)?,
                end: sketch.point(end)?,
            }),
            Geometry::Arc { center, start, end } => Some(Self::Arc {
                center: sketch.point(center)?,
                start: sketch.point(start)?,
                end: sketch.point(end)?,
            }),
            Geometry::Circle { center, radius } => Some(Self::Circle {
                center: sketch.point(center)?,
                radius,
            }),
            Geometry::Point { .. } | Geometry::Spline { .. } => None,
        }
    }

    /// Fits an outline, in sketch coordinates and edge order, to a line, an
    /// arc or a circle.
    ///
    /// # Errors
    ///
    /// Says why when the outline projects to a point or to any other curve.
    pub fn fit(points: &[Point2]) -> Result<Self, String> {
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            return Err(String::from("the edge has no outline"));
        };
        if points.len() < 2 {
            return Err(String::from("the edge has no outline"));
        }
        let (mut low, mut high) = (first, first);
        for p in points {
            low = Point2::new(low.x.min(p.x), low.y.min(p.y));
            high = Point2::new(high.x.max(p.x), high.y.max(p.y));
        }
        if high.x - low.x <= TOLERANCE && high.y - low.y <= TOLERANCE {
            return Err(String::from(
                "the edge is square to the sketch plane, so it projects to a point",
            ));
        }
        let closed = near(first, last);
        if !closed {
            let (dx, dy) = (last.x - first.x, last.y - first.y);
            let length = dx.hypot(dy);
            let straight = points
                .iter()
                .all(|p| ((p.x - first.x) * dy - (p.y - first.y) * dx).abs() / length <= TOLERANCE);
            if straight {
                return Ok(Self::Line {
                    start: first,
                    end: last,
                });
            }
        }
        let count = points.len();
        let pick = |i: usize| points.get(i).copied().unwrap_or(last);
        let (a, b, c) = if closed {
            (first, pick(count / 3), pick(2 * count / 3))
        } else {
            (first, pick(count / 2), last)
        };
        let not_curve = || {
            String::from(
                "the edge does not project to a line, an arc or a circle (a tilted circle gives an ellipse, and splines are not projected yet)",
            )
        };
        let (center, radius) = circumcircle(a, b, c).ok_or_else(not_curve)?;
        if points
            .iter()
            .any(|p| (p.distance(center) - radius).abs() > TOLERANCE)
        {
            return Err(not_curve());
        }
        if closed {
            return Ok(Self::Circle { center, radius });
        }
        // Counter-clockwise when first, middle, last turn left.
        let turn = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        Ok(if turn > 0.0 {
            Self::Arc {
                center,
                start: first,
                end: last,
            }
        } else {
            Self::Arc {
                center,
                start: last,
                end: first,
            }
        })
    }
}

/// The circle through three points, or `None` when they are in a line.
fn circumcircle(a: Point2, b: Point2, c: Point2) -> Option<(Point2, f64)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return None;
    }
    let (a2, b2, c2) = (
        a.x * a.x + a.y * a.y,
        b.x * b.x + b.y * b.y,
        c.x * c.x + c.y * c.y,
    );
    let x = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let y = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let center = Point2::new(x, y);
    let radius = center.distance(a);
    (radius.is_finite()).then_some((center, radius))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn arc_points(radius: f64, from: f64, to: f64, n: usize) -> Vec<Point2> {
        (0..=n)
            .map(|i| {
                let t = from + (to - from) * i as f64 / n as f64;
                Point2::new(5.0 + radius * t.cos(), -2.0 + radius * t.sin())
            })
            .collect()
    }

    #[test]
    fn a_straight_outline_is_a_line() {
        let shape = ProjectedShape::fit(&[Point2::new(0.0, 0.0), Point2::new(30.0, 40.0)]).unwrap();
        assert!(matches!(shape, ProjectedShape::Line { .. }));
    }

    #[test]
    fn a_ring_is_a_circle() {
        let points = arc_points(12.0, 0.0, std::f64::consts::TAU, 48);
        let ProjectedShape::Circle { center, radius } = ProjectedShape::fit(&points).unwrap()
        else {
            panic!("not a circle");
        };
        assert!((radius - 12.0).abs() < 1e-9);
        assert!(center.distance(Point2::new(5.0, -2.0)) < 1e-9);
    }

    #[test]
    fn an_arc_runs_counter_clockwise_whichever_way_the_edge_runs() {
        let mut points = arc_points(10.0, 0.2, 2.0, 20);
        let forward = ProjectedShape::fit(&points).unwrap();
        points.reverse();
        let backward = ProjectedShape::fit(&points).unwrap();
        assert!(forward.matches(&backward));
        let ProjectedShape::Arc { start, end, .. } = forward else {
            panic!("not an arc");
        };
        assert!(
            start.distance(Point2::new(
                5.0 + 10.0 * 0.2f64.cos(),
                -2.0 + 10.0 * 0.2f64.sin()
            )) < 1e-9
        );
        assert!(
            end.distance(Point2::new(
                5.0 + 10.0 * 2.0f64.cos(),
                -2.0 + 10.0 * 2.0f64.sin()
            )) < 1e-9
        );
    }

    #[test]
    fn a_point_and_an_ellipse_are_refused() {
        let point = [Point2::new(1.0, 1.0), Point2::new(1.0, 1.0)];
        assert!(ProjectedShape::fit(&point).unwrap_err().contains("point"));
        let ellipse: Vec<Point2> = (0..=48)
            .map(|i| {
                let t = std::f64::consts::TAU * f64::from(i) / 48.0;
                Point2::new(10.0 * t.cos(), 5.0 * t.sin())
            })
            .collect();
        assert!(
            ProjectedShape::fit(&ellipse)
                .unwrap_err()
                .contains("ellipse")
        );
    }

    #[test]
    fn shapes_match_within_a_small_tolerance_only() {
        let a = ProjectedShape::Circle {
            center: Point2::new(0.0, 0.0),
            radius: 5.0,
        };
        let near_by = ProjectedShape::Circle {
            center: Point2::new(0.0, 0.0),
            radius: 5.0005,
        };
        let moved = ProjectedShape::Circle {
            center: Point2::new(0.0, 0.0),
            radius: 5.1,
        };
        assert!(a.matches(&near_by));
        assert!(!a.matches(&moved));
    }
}
