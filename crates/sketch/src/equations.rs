// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Adapted from CADCraft's `constraints` crate, file `src/residual.rs` at
// commit ca87680fb0f9d7803d3e64c808d0a3f7a196fdaa (https://github.com/storytold/cadcraft).
// Copyright (c) 2026 ArtCraft Team and the CADCraft contributors.
// Licensed under MIT OR Apache-2.0; used here under MIT. See NOTICE.
//
// Changes: rewritten over CNDT's point-based sketch model (lines, circles and
// arcs refer to points) instead of CADCraft's drawing entities; ellipse and
// spline forms dropped; arcs get an implicit equal-radius equation.

//! The residual equations of each constraint. A residual is zero when the
//! constraint holds.
//!
//! Orientation-dependent constraints (tangent side, signed distances, the
//! angle's sense) take their orientation from the geometry when the system is
//! built, so solving never flips a shape inside out.

use crate::ConstraintId;

/// The parameter index of a point's X; its Y follows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct P(pub usize);

/// A line: its start and end points.
#[derive(Debug, Clone, Copy)]
pub(crate) struct L(pub P, pub P);

/// A circle or arc.
#[derive(Debug, Clone, Copy)]
pub(crate) enum C {
    /// A circle: its centre and the index of its radius parameter.
    Circle { center: P, radius: usize },
    /// An arc: its centre and start point (the radius is their distance).
    Arc { center: P, start: P },
}

/// How a constraint was interpreted.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Form {
    Coincident(P, P),
    PointOnLine(P, L),
    PointOnCurve(P, C),
    Midpoint(P, L),
    Horizontal(P, P),
    Vertical(P, P),
    Parallel(L, L),
    Perpendicular(L, L),
    Collinear(L, L),
    EqualLength(L, L),
    EqualRadius(C, C),
    /// `sign` is the side of the line the centre is on.
    TangentLine {
        line: L,
        curve: C,
        sign: f64,
    },
    /// A line and an arc joined at `at`, tangent there: the line runs at
    /// right angles to the radius. Used instead of [`Form::TangentLine`] when
    /// they share an end point, where the distance form is degenerate.
    TangentAt {
        line: L,
        center: P,
        at: P,
    },
    /// Two arcs joined at `at`, tangent there: both centres lie on one line
    /// through it.
    TangentArcsAt {
        a: P,
        b: P,
        at: P,
    },
    /// External or internal tangency; `order` is the sign of `r_a − r_b`.
    TangentCurves {
        a: C,
        b: C,
        external: bool,
        order: f64,
    },
    Concentric(P, P),
    Symmetric(P, P, L),
    Distance {
        a: P,
        b: P,
        value: f64,
    },
    DistanceX {
        a: P,
        b: P,
        sign: f64,
        value: f64,
    },
    DistanceY {
        a: P,
        b: P,
        sign: f64,
        value: f64,
    },
    PointLineDistance {
        point: P,
        line: L,
        sign: f64,
        value: f64,
    },
    Radius(C, f64),
    /// `rev` marks a line whose ray from the corner runs end to start; `sign`
    /// is the sense of the angle; `target` is in radians.
    Angle {
        a: L,
        b: L,
        rev: [bool; 2],
        sign: f64,
        target: f64,
    },
    /// An arc's end stays on its circle (implicit, no constraint behind it).
    ArcEnds {
        center: P,
        start: P,
        end: P,
    },
    /// A fixed point: its parameters are locked, no rows.
    Lock(P),
}

/// One interpreted constraint (or an arc's implicit equation).
#[derive(Debug, Clone)]
pub(crate) struct Equation {
    /// The constraint it comes from; `None` for an arc's implicit equation.
    pub source: Option<ConstraintId>,
    pub form: Form,
}

#[derive(Debug, Clone, Copy)]
struct V2 {
    x: f64,
    y: f64,
}

impl V2 {
    fn sub(self, o: V2) -> V2 {
        V2 {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
    fn dot(self, o: V2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    fn cross(self, o: V2) -> f64 {
        self.x * o.y - self.y * o.x
    }
    fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    fn dist(self, o: V2) -> f64 {
        self.sub(o).len()
    }
    fn mid(self, o: V2) -> V2 {
        V2 {
            x: 0.5 * (self.x + o.x),
            y: 0.5 * (self.y + o.y),
        }
    }
}

/// Unit direction from `a` to `b` (unscaled if degenerate).
fn dir(a: V2, b: V2) -> V2 {
    let d = b.sub(a);
    let l = d.len();
    if l > 1e-12 {
        V2 {
            x: d.x / l,
            y: d.y / l,
        }
    } else {
        d
    }
}

fn ray(a: V2, b: V2, rev: bool) -> V2 {
    if rev { dir(b, a) } else { dir(a, b) }
}

/// +1 or −1; zero counts as positive.
pub(crate) fn sign_of(v: f64) -> f64 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// An angle wrapped into [−π, π).
fn wrap(a: f64) -> f64 {
    let r = (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    if r.is_finite() { r } else { 0.0 }
}

fn pt(x: &[f64], p: P) -> V2 {
    V2 {
        x: x.get(p.0).copied().unwrap_or(0.0),
        y: x.get(p.0 + 1).copied().unwrap_or(0.0),
    }
}

fn line(x: &[f64], l: L) -> (V2, V2) {
    (pt(x, l.0), pt(x, l.1))
}

fn circle(x: &[f64], c: C) -> (V2, f64) {
    match c {
        C::Circle { center, radius } => (pt(x, center), x.get(radius).copied().unwrap_or(0.0)),
        C::Arc { center, start } => {
            let c = pt(x, center);
            (c, c.dist(pt(x, start)))
        }
    }
}

/// Where two infinite lines cross, as fractions along each: `(t, u)`.
fn crossing(a: (V2, V2), b: (V2, V2)) -> Option<(f64, f64)> {
    let (d1, d2) = (a.1.sub(a.0), b.1.sub(b.0));
    let denom = d1.cross(d2);
    if denom.abs() <= 1e-12 * d1.len() * d2.len() {
        return None;
    }
    let w = b.0.sub(a.0);
    Some((w.cross(d2) / denom, w.cross(d1) / denom))
}

impl Form {
    /// How many residuals this form produces.
    pub(crate) fn rows(&self) -> usize {
        match self {
            Form::Lock(_) => 0,
            Form::Coincident(..)
            | Form::Midpoint(..)
            | Form::Collinear(..)
            | Form::Concentric(..)
            | Form::Symmetric(..) => 2,
            _ => 1,
        }
    }

    /// Settles orientation-dependent forms from the starting geometry `x`.
    pub(crate) fn orient(self, x: &[f64]) -> Form {
        match self {
            Form::TangentLine { line: l, curve, .. } => {
                let (a, b) = line(x, l);
                let (c, _) = circle(x, curve);
                Form::TangentLine {
                    line: l,
                    curve,
                    sign: sign_of(dir(a, b).cross(c.sub(a))),
                }
            }
            Form::TangentCurves { a, b, .. } => {
                let ((ca, ra), (cb, rb)) = (circle(x, a), circle(x, b));
                let external = ca.dist(cb) >= ra.abs().max(rb.abs());
                Form::TangentCurves {
                    a,
                    b,
                    external,
                    order: sign_of(ra - rb),
                }
            }
            Form::DistanceX { a, b, value, .. } => Form::DistanceX {
                a,
                b,
                value,
                sign: sign_of(pt(x, b).x - pt(x, a).x),
            },
            Form::DistanceY { a, b, value, .. } => Form::DistanceY {
                a,
                b,
                value,
                sign: sign_of(pt(x, b).y - pt(x, a).y),
            },
            Form::PointLineDistance {
                point,
                line: l,
                value,
                ..
            } => {
                let (a, b) = line(x, l);
                let sign = sign_of(dir(a, b).cross(pt(x, point).sub(a)));
                Form::PointLineDistance {
                    point,
                    line: l,
                    value,
                    sign,
                }
            }
            Form::Angle { a, b, target, .. } => {
                let (la, lb) = (line(x, a), line(x, b));
                // Measure along the rays from the corner to each line's far
                // end, the angle a dimension draws, so a line drawn towards
                // the corner does not measure the supplement.
                let rev = crossing(la, lb).map_or([false, false], |(t, u)| [t > 0.5, u > 0.5]);
                let (d1, d2) = (ray(la.0, la.1, rev[0]), ray(lb.0, lb.1, rev[1]));
                let sign = sign_of(d1.cross(d2).atan2(d1.dot(d2)));
                Form::Angle {
                    a,
                    b,
                    rev,
                    sign,
                    target,
                }
            }
            other => other,
        }
    }

    /// Appends exactly [`Form::rows`] residuals for parameter values `x`.
    pub(crate) fn eval(&self, x: &[f64], out: &mut Vec<f64>) {
        match *self {
            Form::Lock(_) => {}
            Form::Coincident(a, b) | Form::Concentric(a, b) => {
                let (a, b) = (pt(x, a), pt(x, b));
                out.extend([a.x - b.x, a.y - b.y]);
            }
            Form::PointOnLine(p, l) => {
                let (a, b) = line(x, l);
                out.push(dir(a, b).cross(pt(x, p).sub(a)));
            }
            Form::PointOnCurve(p, c) => {
                let (c, r) = circle(x, c);
                out.push(pt(x, p).dist(c) - r);
            }
            Form::Midpoint(p, l) => {
                let (a, b) = line(x, l);
                let (p, m) = (pt(x, p), a.mid(b));
                out.extend([p.x - m.x, p.y - m.y]);
            }
            Form::Horizontal(a, b) => out.push(pt(x, a).y - pt(x, b).y),
            Form::Vertical(a, b) => out.push(pt(x, a).x - pt(x, b).x),
            Form::Parallel(l1, l2) => {
                let ((a1, b1), (a2, b2)) = (line(x, l1), line(x, l2));
                out.push(dir(a1, b1).cross(dir(a2, b2)));
            }
            Form::Perpendicular(l1, l2) => {
                let ((a1, b1), (a2, b2)) = (line(x, l1), line(x, l2));
                out.push(dir(a1, b1).dot(dir(a2, b2)));
            }
            Form::Collinear(l1, l2) => {
                let ((a1, b1), (a2, b2)) = (line(x, l1), line(x, l2));
                let d = dir(a1, b1);
                out.extend([d.cross(a2.sub(a1)), d.cross(b2.sub(a1))]);
            }
            Form::EqualLength(l1, l2) => {
                let ((a1, b1), (a2, b2)) = (line(x, l1), line(x, l2));
                out.push(a1.dist(b1) - a2.dist(b2));
            }
            Form::EqualRadius(c1, c2) => out.push(circle(x, c1).1 - circle(x, c2).1),
            Form::TangentLine {
                line: l,
                curve,
                sign,
            } => {
                let (a, b) = line(x, l);
                let (c, r) = circle(x, curve);
                out.push(sign * dir(a, b).cross(c.sub(a)) - r);
            }
            Form::TangentAt {
                line: l,
                center,
                at,
            } => {
                let (a, b) = line(x, l);
                out.push(dir(a, b).dot(dir(pt(x, center), pt(x, at))));
            }
            Form::TangentArcsAt { a, b, at } => {
                let p = pt(x, at);
                out.push(dir(pt(x, a), p).cross(dir(pt(x, b), p)));
            }
            Form::TangentCurves {
                a,
                b,
                external,
                order,
            } => {
                let ((ca, ra), (cb, rb)) = (circle(x, a), circle(x, b));
                let d = ca.dist(cb);
                out.push(if external {
                    d - (ra + rb)
                } else {
                    d - order * (ra - rb)
                });
            }
            Form::Symmetric(p1, p2, l) => {
                let (p1, p2) = (pt(x, p1), pt(x, p2));
                let (a, b) = line(x, l);
                let d = dir(a, b);
                out.extend([d.cross(p1.mid(p2).sub(a)), d.dot(p2.sub(p1))]);
            }
            Form::Distance { a, b, value } => out.push(pt(x, a).dist(pt(x, b)) - value),
            Form::DistanceX { a, b, sign, value } => {
                out.push(sign * (pt(x, b).x - pt(x, a).x) - value);
            }
            Form::DistanceY { a, b, sign, value } => {
                out.push(sign * (pt(x, b).y - pt(x, a).y) - value);
            }
            Form::PointLineDistance {
                point,
                line: l,
                sign,
                value,
            } => {
                let (a, b) = line(x, l);
                out.push(sign * dir(a, b).cross(pt(x, point).sub(a)) - value);
            }
            Form::Radius(c, value) => out.push(circle(x, c).1 - value),
            Form::Angle {
                a,
                b,
                rev,
                sign,
                target,
            } => {
                let ((a1, b1), (a2, b2)) = (line(x, a), line(x, b));
                let (d1, d2) = (ray(a1, b1, rev[0]), ray(a2, b2, rev[1]));
                out.push(wrap(d1.cross(d2).atan2(d1.dot(d2)) - sign * target));
            }
            Form::ArcEnds { center, start, end } => {
                let c = pt(x, center);
                out.push(c.dist(pt(x, start)) - c.dist(pt(x, end)));
            }
        }
    }
}

impl Equation {
    pub(crate) fn rows(&self) -> usize {
        self.form.rows()
    }

    pub(crate) fn eval(&self, x: &[f64], out: &mut Vec<f64>) {
        let start = out.len();
        self.form.eval(x, out);
        out.resize(start + self.rows(), 0.0);
    }
}
