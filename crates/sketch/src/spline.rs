// SPDX-License-Identifier: AGPL-3.0-or-later
//! Points along a fit-point spline.
//!
//! The kernel draws a spline as a cubic B-spline interpolating its points. The
//! sketch only needs a close picture of it, to find regions and which one
//! lies inside which, so it uses the centripetal Catmull-Rom curve through the
//! same points: also cubic, smooth, and free of loops and cusps.

use cndt_geom::Point2;

/// Points along the spline through `points`, `per_span` for each stretch
/// between two neighbouring points, starting at the first point. A closed
/// spline returns to the first point and stops short of repeating it; an open
/// one ends on the last point.
#[must_use]
pub fn spline_samples(points: &[Point2], closed: bool, per_span: usize) -> Vec<Point2> {
    let n = points.len();
    if n < 2 || per_span == 0 {
        return points.to_vec();
    }
    let reflect = |a: Point2, b: Point2| Point2::new(2.0 * a.x - b.x, 2.0 * a.y - b.y);
    let at = |i: isize| -> Point2 {
        if closed {
            points[i.rem_euclid(n as isize) as usize]
        } else if i < 0 {
            // Reflect the first neighbour to give the end a slope.
            reflect(points[0], points[1])
        } else if i as usize >= n {
            reflect(points[n - 1], points[n - 2])
        } else {
            points[i as usize]
        }
    };
    let spans = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(spans * per_span + 1);
    for s in 0..spans as isize {
        let (p0, p1, p2, p3) = (at(s - 1), at(s), at(s + 1), at(s + 2));
        for k in 0..per_span {
            out.push(centripetal(p0, p1, p2, p3, k as f64 / per_span as f64));
        }
    }
    if !closed {
        out.push(points[n - 1]);
    }
    out
}

/// The point at fraction `t` of the span from `p1` to `p2` (Barry and
/// Goldman's pyramid form of the centripetal Catmull-Rom spline).
fn centripetal(p0: Point2, p1: Point2, p2: Point2, p3: Point2, t: f64) -> Point2 {
    let knot = |a: Point2, b: Point2| a.distance(b).sqrt().max(1e-9);
    let t0 = 0.0;
    let t1 = t0 + knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let u = t1 + (t2 - t1) * t;
    let lerp = |a: Point2, b: Point2, ta: f64, tb: f64| {
        let w = (u - ta) / (tb - ta);
        Point2::new(a.x + (b.x - a.x) * w, a.y + (b.y - a.y) * w)
    };
    let a1 = lerp(p0, p1, t0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, t0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    fn circle_points(n: usize) -> Vec<Point2> {
        (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                Point2::new(10.0 * a.cos(), 10.0 * a.sin())
            })
            .collect()
    }

    #[test]
    fn open_spline_passes_through_its_points() {
        let pts = [
            Point2::new(0.0, 0.0),
            Point2::new(5.0, 4.0),
            Point2::new(10.0, 0.0),
        ];
        let out = spline_samples(&pts, false, 8);
        assert_eq!(out.len(), 17);
        assert!(out[0].distance(pts[0]) < 1e-9);
        assert!(out[8].distance(pts[1]) < 1e-9);
        assert!(out[16].distance(pts[2]) < 1e-9);
    }

    #[test]
    fn closed_spline_round_a_circle_stays_close_to_it() {
        let out = spline_samples(&circle_points(8), true, 8);
        assert_eq!(out.len(), 64);
        for p in &out {
            assert!(
                (p.distance(Point2::new(0.0, 0.0)) - 10.0).abs() < 0.2,
                "{p:?}"
            );
        }
    }

    #[test]
    fn too_few_points_come_back_unchanged() {
        let pts = [Point2::new(1.0, 2.0)];
        assert_eq!(spline_samples(&pts, false, 8), pts.to_vec());
    }
}
