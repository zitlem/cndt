// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Adapted from CADCraft's `constraints` crate, files `src/lib.rs` (components,
// solving, conflict and redundancy reporting) and `src/dof.rs` (degrees of
// freedom) at commit ca87680fb0f9d7803d3e64c808d0a3f7a196fdaa
// (https://github.com/storytold/cadcraft).
// Copyright (c) 2026 ArtCraft Team and the CADCraft contributors.
// Licensed under MIT OR Apache-2.0; used here under MIT. See NOTICE.
//
// Changes: rewritten over CNDT's point-based sketch model; HashMap and
// HashSet replaced with ordered collections (CNDT rule 11); spline, block and
// expression handling dropped.

//! Constraint systems: the sketch's equations split into independent
//! components, each solved on its own.
//!
//! The parameters are the coordinates of points and the radii of circles.
//! Two equations belong to the same component when they share a parameter.

use std::collections::{BTreeMap, BTreeSet};

use crate::equations::{C, Equation, Form, L, P};
use crate::solver::{self, Outcome};
use crate::{
    ConstraintId, EntityFreedom, EntityId, Freedom, Geometry, Point2, SketchError, SolveOptions,
    Status,
};

/// Constraints tried one by one when narrowing down a conflict.
const MAX_CONFLICT_PROBES: usize = 48;
/// Solver iterations.
const MAX_ITER: usize = 200;
/// Most free parameters for which per-entity freedom is analysed.
const MAX_NULL_COLS: usize = 600;
/// Largest Jacobian (rows × columns × min) whose rank is computed.
const MAX_RANK_WORK: f64 = 4e8;
/// Weight of the parameters of points the user is working on.
const KEEP_WEIGHT: f64 = 1e4;

/// The parameter owners of an entity: points (x, y) and circles (radius).
fn owners(sketch: &crate::Sketch, id: EntityId) -> Vec<EntityId> {
    match sketch.entity(id).map(|e| &e.geometry) {
        Some(Geometry::Point { .. }) => vec![id],
        Some(Geometry::Line { start, end }) => vec![*start, *end],
        Some(Geometry::Circle { center, .. }) => vec![*center, id],
        Some(Geometry::Arc { center, start, end }) => vec![*center, *start, *end],
        None => Vec::new(),
    }
}

/// Number of parameters an owner has.
fn param_count(sketch: &crate::Sketch, owner: EntityId) -> usize {
    match sketch.entity(owner).map(|e| &e.geometry) {
        Some(Geometry::Point { .. }) => 2,
        Some(Geometry::Circle { .. }) => 1,
        _ => 0,
    }
}

/// Something that produces equations: a constraint, or an arc's implicit
/// equation.
#[derive(Clone, Copy)]
enum Source {
    Constraint(ConstraintId),
    Arc(EntityId),
}

/// One independent group of equations and its parameters.
pub(crate) struct Component {
    /// Parameter offset of each owner.
    offsets: BTreeMap<EntityId, usize>,
    x0: Vec<f64>,
    equations: Vec<Equation>,
    weights: Vec<f64>,
    tol: f64,
}

/// Resolves entity references to parameter positions.
struct Resolver<'a> {
    sketch: &'a crate::Sketch,
    offsets: &'a BTreeMap<EntityId, usize>,
}

impl Resolver<'_> {
    fn point(&self, id: EntityId) -> Result<P, SketchError> {
        self.offsets
            .get(&id)
            .map(|&o| P(o))
            .ok_or(SketchError::UnknownEntity(id))
    }

    fn line(&self, id: EntityId) -> Result<L, SketchError> {
        match self.sketch.entity(id).map(|e| &e.geometry) {
            Some(Geometry::Line { start, end }) => Ok(L(self.point(*start)?, self.point(*end)?)),
            _ => Err(SketchError::UnknownEntity(id)),
        }
    }

    fn curve(&self, id: EntityId) -> Result<C, SketchError> {
        match self.sketch.entity(id).map(|e| &e.geometry) {
            Some(Geometry::Circle { center, .. }) => Ok(C::Circle {
                center: self.point(*center)?,
                radius: *self
                    .offsets
                    .get(&id)
                    .ok_or(SketchError::UnknownEntity(id))?,
            }),
            Some(Geometry::Arc { center, start, .. }) => Ok(C::Arc {
                center: self.point(*center)?,
                start: self.point(*start)?,
            }),
            _ => Err(SketchError::UnknownEntity(id)),
        }
    }

    fn center(&self, id: EntityId) -> Result<P, SketchError> {
        match self.sketch.entity(id).map(|e| &e.geometry) {
            Some(Geometry::Circle { center, .. } | Geometry::Arc { center, .. }) => {
                self.point(*center)
            }
            _ => Err(SketchError::UnknownEntity(id)),
        }
    }

    /// An end point shared by a line or arc and an arc, if any.
    fn shared_end(&self, a: EntityId, b: EntityId) -> Option<EntityId> {
        let ends = |id: EntityId| match self.sketch.entity(id).map(|e| &e.geometry) {
            Some(Geometry::Line { start, end } | Geometry::Arc { start, end, .. }) => {
                vec![*start, *end]
            }
            _ => Vec::new(),
        };
        let theirs = ends(b);
        ends(a).into_iter().find(|p| theirs.contains(p))
    }

    fn is_line(&self, id: EntityId) -> bool {
        matches!(
            self.sketch.entity(id).map(|e| &e.geometry),
            Some(Geometry::Line { .. })
        )
    }

    /// The equation form of a source, before orientation.
    fn form(&self, source: Source) -> Result<Form, SketchError> {
        use crate::Axis;
        use crate::Constraint as K;
        let id = match source {
            Source::Arc(arc) => {
                return match self.sketch.entity(arc).map(|e| &e.geometry) {
                    Some(Geometry::Arc { center, start, end }) => Ok(Form::ArcEnds {
                        center: self.point(*center)?,
                        start: self.point(*start)?,
                        end: self.point(*end)?,
                    }),
                    _ => Err(SketchError::UnknownEntity(arc)),
                };
            }
            Source::Constraint(id) => id,
        };
        let constraint = self
            .sketch
            .constraint(id)
            .ok_or(SketchError::UnknownConstraint(id))?;
        Ok(match *constraint {
            K::Coincident { a, b } => Form::Coincident(self.point(a)?, self.point(b)?),
            K::PointOnLine { point, line } => {
                Form::PointOnLine(self.point(point)?, self.line(line)?)
            }
            K::PointOnCurve { point, curve } => {
                Form::PointOnCurve(self.point(point)?, self.curve(curve)?)
            }
            K::Midpoint { point, line } => Form::Midpoint(self.point(point)?, self.line(line)?),
            K::Horizontal { line } => {
                let l = self.line(line)?;
                Form::Horizontal(l.0, l.1)
            }
            K::Vertical { line } => {
                let l = self.line(line)?;
                Form::Vertical(l.0, l.1)
            }
            K::HorizontalPoints { a, b } => Form::Horizontal(self.point(a)?, self.point(b)?),
            K::VerticalPoints { a, b } => Form::Vertical(self.point(a)?, self.point(b)?),
            K::Parallel { a, b } => Form::Parallel(self.line(a)?, self.line(b)?),
            K::Perpendicular { a, b } => Form::Perpendicular(self.line(a)?, self.line(b)?),
            K::Collinear { a, b } => Form::Collinear(self.line(a)?, self.line(b)?),
            K::Equal { a, b } if self.is_line(a) => Form::EqualLength(self.line(a)?, self.line(b)?),
            K::Equal { a, b } => Form::EqualRadius(self.curve(a)?, self.curve(b)?),
            K::Tangent { a, b } => match (self.shared_end(a, b), self.is_line(a)) {
                (Some(at), true) => Form::TangentAt {
                    line: self.line(a)?,
                    center: self.center(b)?,
                    at: self.point(at)?,
                },
                (Some(at), false) => Form::TangentArcsAt {
                    a: self.center(a)?,
                    b: self.center(b)?,
                    at: self.point(at)?,
                },
                (None, true) => Form::TangentLine {
                    line: self.line(a)?,
                    curve: self.curve(b)?,
                    sign: 1.0,
                },
                (None, false) => Form::TangentCurves {
                    a: self.curve(a)?,
                    b: self.curve(b)?,
                    external: true,
                    order: 1.0,
                },
            },
            K::Concentric { a, b } => Form::Concentric(self.center(a)?, self.center(b)?),
            K::Symmetric { a, b, axis } => {
                Form::Symmetric(self.point(a)?, self.point(b)?, self.line(axis)?)
            }
            K::Fix { point } => Form::Lock(self.point(point)?),
            K::Distance { a, b, axis, value } => {
                let (a, b) = (self.point(a)?, self.point(b)?);
                match axis {
                    Axis::Aligned => Form::Distance { a, b, value },
                    Axis::Horizontal => Form::DistanceX {
                        a,
                        b,
                        sign: 1.0,
                        value,
                    },
                    Axis::Vertical => Form::DistanceY {
                        a,
                        b,
                        sign: 1.0,
                        value,
                    },
                }
            }
            K::PointLineDistance { point, line, value } => Form::PointLineDistance {
                point: self.point(point)?,
                line: self.line(line)?,
                sign: 1.0,
                value,
            },
            K::Length { line, value } => {
                let l = self.line(line)?;
                Form::Distance {
                    a: l.0,
                    b: l.1,
                    value,
                }
            }
            K::Radius { curve, value } => Form::Radius(self.curve(curve)?, value),
            K::Diameter { curve, value } => Form::Radius(self.curve(curve)?, value / 2.0),
            K::Angle { a, b, value } => Form::Angle {
                a: self.line(a)?,
                b: self.line(b)?,
                rev: [false, false],
                sign: 1.0,
                target: value.to_radians(),
            },
        })
    }
}

/// Every equation source with the parameter owners it touches.
fn sources(sketch: &crate::Sketch) -> Vec<(Source, Vec<EntityId>)> {
    let mut out = Vec::new();
    for (id, entity) in sketch.entities() {
        if matches!(entity.geometry, Geometry::Arc { .. }) {
            out.push((Source::Arc(id), owners(sketch, id)));
        }
    }
    for (id, constraint) in sketch.constraints() {
        let touched = constraint
            .entities()
            .into_iter()
            .flat_map(|e| owners(sketch, e))
            .collect();
        out.push((Source::Constraint(id), touched));
    }
    out
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let grand = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = grand;
        }
        i = p;
    }
    i
}

/// Splits the sketch's equations into independent components.
pub(crate) fn components(
    sketch: &crate::Sketch,
    options: &SolveOptions,
) -> Result<Vec<Component>, SketchError> {
    let sources = sources(sketch);
    let mut parent: Vec<usize> = (0..sources.len()).collect();
    let mut first_user: BTreeMap<EntityId, usize> = BTreeMap::new();
    for (i, (_, touched)) in sources.iter().enumerate() {
        for owner in touched {
            match first_user.get(owner) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b
                        && let Some(slot) = parent.get_mut(a)
                    {
                        *slot = b;
                    }
                }
                None => {
                    first_user.insert(*owner, i);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..sources.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let mut out = Vec::new();
    for members in groups.into_values() {
        let touched: BTreeSet<EntityId> = members
            .iter()
            .filter_map(|&i| sources.get(i))
            .flat_map(|(_, t)| t.iter().copied())
            .collect();
        let mut offsets = BTreeMap::new();
        let mut x0 = Vec::new();
        let mut weights = Vec::new();
        for owner in touched {
            offsets.insert(owner, x0.len());
            match sketch.entity(owner).map(|e| &e.geometry) {
                Some(Geometry::Point { at }) => {
                    x0.extend([at.x, at.y]);
                    let w = if options.keep.contains(&owner) {
                        KEEP_WEIGHT
                    } else {
                        1.0
                    };
                    weights.extend([w, w]);
                }
                Some(Geometry::Circle { radius, .. }) => {
                    x0.push(*radius);
                    weights.push(1.0);
                }
                _ => {}
            }
        }
        let resolver = Resolver {
            sketch,
            offsets: &offsets,
        };
        let mut equations = Vec::with_capacity(members.len());
        for &i in &members {
            let Some((source, _)) = sources.get(i) else {
                continue;
            };
            let form = resolver.form(*source)?.orient(&x0);
            let source = match source {
                Source::Constraint(id) => Some(*id),
                Source::Arc(_) => None,
            };
            equations.push(Equation { source, form });
        }
        let tol = 1e-9 * x0.iter().fold(1.0f64, |m, v| m.max(v.abs()));
        out.push(Component {
            offsets,
            x0,
            equations,
            weights,
            tol,
        });
    }
    Ok(out)
}

impl Component {
    fn contains(&self, id: ConstraintId) -> bool {
        self.equations.iter().any(|e| e.source == Some(id))
    }

    fn locked(&self, skip: Option<usize>) -> Vec<bool> {
        let mut locked = vec![false; self.x0.len()];
        for (i, e) in self.equations.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            if let Form::Lock(p) = e.form {
                for j in [p.0, p.0 + 1] {
                    if let Some(l) = locked.get_mut(j) {
                        *l = true;
                    }
                }
            }
        }
        locked
    }

    fn free(&self, skip: Option<usize>) -> Vec<usize> {
        self.locked(skip)
            .iter()
            .enumerate()
            .filter(|(_, l)| !**l)
            .map(|(i, _)| i)
            .collect()
    }

    fn rows(&self, skip: Option<usize>) -> usize {
        self.equations
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != skip)
            .map(|(_, e)| e.rows())
            .sum()
    }

    fn eval(&self, x: &[f64], out: &mut Vec<f64>, skip: Option<usize>) {
        for (i, e) in self.equations.iter().enumerate() {
            if Some(i) != skip {
                e.eval(x, out);
            }
        }
    }

    fn run(&self, skip: Option<usize>) -> Result<Outcome, SketchError> {
        let free = self.free(skip);
        let rows = self.rows(skip);
        if rows > solver::MAX_ROWS || (rows as f64) * (rows as f64) * (free.len() as f64) > 4e8 {
            return Err(SketchError::TooLarge);
        }
        let f = |x: &[f64], out: &mut Vec<f64>| self.eval(x, out, skip);
        Ok(solver::solve(
            &f,
            &self.x0,
            &free,
            &self.weights,
            self.tol,
            MAX_ITER,
        ))
    }

    /// The constraints in conflict: each one whose removal lets the rest
    /// solve, or failing that, the ones left unsatisfied.
    fn conflicts(&self, x_failed: &[f64]) -> Vec<ConstraintId> {
        let mut ids = Vec::new();
        if self.equations.len() <= MAX_CONFLICT_PROBES {
            for (i, e) in self.equations.iter().enumerate() {
                let Some(id) = e.source else { continue };
                if self.run(Some(i)).is_ok_and(|o| o.converged) {
                    ids.push(id);
                }
            }
        }
        if ids.is_empty() {
            for e in &self.equations {
                let mut out = Vec::new();
                e.eval(x_failed, &mut out);
                if let Some(id) = e.source
                    && solver::max_abs(&out) > self.tol * 10.0
                {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// Constraints that share an owner with equation `index`.
    fn neighbours(&self, sketch: &crate::Sketch, index: usize) -> Vec<ConstraintId> {
        let touched = |e: &Equation| -> BTreeSet<EntityId> {
            e.source
                .and_then(|id| sketch.constraint(id))
                .map(|c| {
                    c.entities()
                        .into_iter()
                        .flat_map(|x| owners(sketch, x))
                        .collect()
                })
                .unwrap_or_default()
        };
        let Some(mine) = self.equations.get(index).map(touched) else {
            return Vec::new();
        };
        self.equations
            .iter()
            .enumerate()
            .filter(|(i, e)| *i != index && !touched(e).is_disjoint(&mine))
            .filter_map(|(_, e)| e.source)
            .collect()
    }

    fn write_back(&self, sketch: &mut crate::Sketch, x: &[f64]) {
        for (owner, &offset) in &self.offsets {
            let Some(entity) = sketch.entities.get_mut(owner) else {
                continue;
            };
            match &mut entity.geometry {
                Geometry::Point { at } => {
                    if let (Some(&px), Some(&py)) = (x.get(offset), x.get(offset + 1)) {
                        *at = Point2::new(px, py);
                    }
                }
                Geometry::Circle { radius, .. } => {
                    if let Some(&r) = x.get(offset) {
                        *radius = r;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Solves every component and writes the geometry back. On error the
/// sketch may be partly changed, so callers solve a copy.
pub(crate) fn solve(sketch: &mut crate::Sketch, options: &SolveOptions) -> Result<(), SketchError> {
    let mut solutions = Vec::new();
    for component in components(sketch, options)? {
        let outcome = component.run(None)?;
        if !outcome.converged {
            return Err(SketchError::Conflict {
                constraints: component.conflicts(&outcome.x),
            });
        }
        solutions.push((component, outcome.x));
    }
    for (component, x) in solutions {
        component.write_back(sketch, &x);
    }
    Ok(())
}

/// Solves the component holding the newly added constraint `id`, refusing
/// it if it conflicts with or duplicates the existing constraints.
pub(crate) fn add_checked(
    sketch: &mut crate::Sketch,
    id: ConstraintId,
    options: &SolveOptions,
) -> Result<(), SketchError> {
    let component = components(sketch, options)?
        .into_iter()
        .find(|c| c.contains(id))
        .ok_or(SketchError::UnknownConstraint(id))?;
    let outcome = component.run(None)?;
    if !outcome.converged {
        let mut ids = component.conflicts(&outcome.x);
        ids.retain(|i| *i != id);
        ids.insert(0, id);
        return Err(SketchError::Conflict { constraints: ids });
    }
    let index = component
        .equations
        .iter()
        .position(|e| e.source == Some(id))
        .ok_or(SketchError::UnknownConstraint(id))?;
    let new = component
        .equations
        .get(index)
        .ok_or(SketchError::UnknownConstraint(id))?;
    if new.rows() > 0 {
        redundancy(sketch, &component, index, &outcome.x)?;
    }
    if let Form::Lock(_) = new.form {
        lock_redundancy(sketch, &component, index, &outcome.x)?;
    }
    component.write_back(sketch, &outcome.x);
    Ok(())
}

/// Refuses equation `index` if its rows do not raise the Jacobian's rank.
fn redundancy(
    sketch: &crate::Sketch,
    component: &Component,
    index: usize,
    x: &[f64],
) -> Result<(), SketchError> {
    let free = component.free(None);
    let nf = free.len();
    // The new equation last, so its rows are the bottom of the Jacobian.
    let mut order: Vec<&Equation> = component
        .equations
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, e)| e)
        .collect();
    let Some(new) = component.equations.get(index) else {
        return Ok(());
    };
    order.push(new);
    let f = |x: &[f64], out: &mut Vec<f64>| {
        for e in &order {
            e.eval(x, out);
        }
    };
    let rows_all: usize = order.iter().map(|e| e.rows()).sum();
    let rows_old = rows_all - new.rows();
    let jac = solver::jacobian(&f, x, &free, rows_all);
    let rank_all = solver::rank(&jac, rows_all, nf);
    let rank_old = solver::rank(jac.get(..rows_old * nf).unwrap_or_default(), rows_old, nf);
    if rank_all >= rank_old + new.rows() {
        return Ok(());
    }
    // Which existing constraints it duplicates: each one whose removal would
    // let the new one add independent equations.
    let mut starts = Vec::with_capacity(order.len());
    let mut at = 0;
    for e in &order {
        starts.push(at);
        at += e.rows();
    }
    let pick = |skip: usize, with_new: bool| -> (Vec<f64>, usize) {
        let mut m = Vec::new();
        let mut rows = 0;
        let last = order.len().saturating_sub(1);
        for (i, e) in order.iter().enumerate() {
            if i == skip || (i == last && !with_new) {
                continue;
            }
            let s = starts.get(i).copied().unwrap_or(0);
            if let Some(part) = jac.get(s * nf..(s + e.rows()) * nf) {
                m.extend_from_slice(part);
                rows += e.rows();
            }
        }
        (m, rows)
    };
    let mut ids = Vec::new();
    if order.len() <= MAX_CONFLICT_PROBES + 1 {
        for (i, e) in order.iter().enumerate().take(order.len().saturating_sub(1)) {
            let Some(source) = e.source else { continue };
            if e.rows() == 0 {
                continue;
            }
            let (old, ro) = pick(i, false);
            let (all, ra) = pick(i, true);
            if solver::rank(&all, ra, nf) == solver::rank(&old, ro, nf) + new.rows() {
                ids.push(source);
            }
        }
    }
    if ids.is_empty() {
        ids = component.neighbours(sketch, index);
    }
    Err(SketchError::Redundant { constraints: ids })
}

/// Refuses a Fix whose point is already held by the other constraints.
fn lock_redundancy(
    sketch: &crate::Sketch,
    component: &Component,
    index: usize,
    x: &[f64],
) -> Result<(), SketchError> {
    let before = component.free(Some(index));
    let after = component.free(None);
    let locked: Vec<usize> = before
        .iter()
        .enumerate()
        .filter(|(_, j)| !after.contains(j))
        .map(|(k, _)| k)
        .collect();
    let rows_old = component.rows(Some(index));
    let nf = before.len();
    let f = |x: &[f64], out: &mut Vec<f64>| component.eval(x, out, Some(index));
    let mut jac = solver::jacobian(&f, x, &before, rows_old);
    let rank_old = solver::rank(&jac, rows_old, nf);
    for k in &locked {
        jac.extend((0..nf).map(|c| if c == *k { 1.0 } else { 0.0 }));
    }
    if locked.is_empty()
        || solver::rank(&jac, rows_old + locked.len(), nf) < rank_old + locked.len()
    {
        return Err(SketchError::Redundant {
            constraints: component.neighbours(sketch, index),
        });
    }
    Ok(())
}

/// Freedom of one component: its remaining degrees of freedom, its redundant
/// equations, and the null-space basis over its free parameters.
struct Analysis {
    dof: Option<usize>,
    redundant: usize,
    free: Vec<usize>,
    basis: Option<Vec<Vec<f64>>>,
}

fn analyse(component: &Component) -> Analysis {
    let free = component.free(None);
    let rows = component.rows(None);
    let nf = free.len();
    if rows > solver::MAX_ROWS
        || (rows as f64) * (nf as f64) * (rows.min(nf) as f64) > MAX_RANK_WORK
    {
        return Analysis {
            dof: None,
            redundant: 0,
            free,
            basis: None,
        };
    }
    let f = |x: &[f64], out: &mut Vec<f64>| component.eval(x, out, None);
    let jac = solver::jacobian(&f, &component.x0, &free, rows);
    if nf > MAX_NULL_COLS {
        let rank = solver::rank(&jac, rows, nf);
        return Analysis {
            dof: Some(nf - rank.min(nf)),
            redundant: rows.saturating_sub(rank),
            free,
            basis: None,
        };
    }
    let (rank, basis) = solver::null_space(&jac, rows, nf);
    Analysis {
        dof: Some(nf - rank.min(nf)),
        redundant: rows.saturating_sub(rank),
        free,
        basis: Some(basis),
    }
}

/// Degrees of freedom of the whole sketch and of every entity.
pub(crate) fn freedom(sketch: &crate::Sketch) -> Freedom {
    let Ok(components) = components(sketch, &SolveOptions::default()) else {
        return Freedom {
            dof: None,
            redundant: 0,
            status: Status::Unknown,
            entities: Vec::new(),
        };
    };
    let analyses: Vec<Analysis> = components.iter().map(analyse).collect();
    let mut total = Some(0usize);
    let mut redundant = 0;
    for a in &analyses {
        redundant += a.redundant;
        total = total.zip(a.dof).map(|(t, d)| t + d);
    }
    // Parameters no equation touches are entirely free.
    let in_component = |owner: &EntityId| components.iter().any(|c| c.offsets.contains_key(owner));
    for (id, _) in sketch.entities() {
        if !in_component(&id) {
            total = total.map(|t| t + param_count(sketch, id));
        }
    }
    let mut entities = Vec::new();
    for (id, _) in sketch.entities() {
        let mut dof = Some(0usize);
        let mut over = false;
        for owner in owners(sketch, id) {
            if !in_component(&owner) {
                dof = dof.map(|d| d + param_count(sketch, owner));
            }
        }
        for (component, analysis) in components.iter().zip(&analyses) {
            // This entity's free columns in this component.
            let mut params = Vec::new();
            for owner in owners(sketch, id) {
                if let Some(&offset) = component.offsets.get(&owner) {
                    params.extend(offset..offset + param_count(sketch, owner));
                }
            }
            if params.is_empty() {
                continue;
            }
            over |= analysis.redundant > 0;
            let cols: Vec<usize> = analysis
                .free
                .iter()
                .enumerate()
                .filter(|(_, j)| params.contains(j))
                .map(|(k, _)| k)
                .collect();
            let part = match &analysis.basis {
                _ if cols.is_empty() => Some(0),
                Some(basis) if basis.is_empty() => Some(0),
                Some(basis) => {
                    let restricted: Vec<Vec<f64>> = basis
                        .iter()
                        .map(|v| {
                            cols.iter()
                                .map(|&k| v.get(k).copied().unwrap_or(0.0))
                                .collect()
                        })
                        .collect();
                    Some(solver::qr(restricted, 0.0, 1e-6).0)
                }
                None => None,
            };
            dof = dof.zip(part).map(|(d, p)| d + p);
        }
        let status = match dof {
            _ if over => Status::Over,
            Some(0) => Status::Fully,
            Some(_) => Status::Under,
            None => Status::Unknown,
        };
        entities.push(EntityFreedom { id, dof, status });
    }
    let status = match total {
        _ if redundant > 0 => Status::Over,
        Some(0) => Status::Fully,
        Some(_) => Status::Under,
        None => Status::Unknown,
    };
    Freedom {
        dof: total,
        redundant,
        status,
        entities,
    }
}
