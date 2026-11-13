// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 1: 2D sketches and the geometric constraint solver.
//!
//! A [`Sketch`] holds points, lines, circles and arcs on a plane, and the
//! constraints between them: coincident, horizontal, tangent, distance and
//! the rest. Points are the only entities with a position of their own. Lines,
//! circles and arcs refer to points, so a rectangle's corners are shared
//! points, as in a Fusion 360 sketch.
//!
//! The solver keeps every constraint satisfied and moves the geometry as
//! little as possible. It never fails silently:
//!
//! - A constraint that cannot hold is refused, and the error names the
//!   existing constraints it conflicts with ([`SketchError::Conflict`]).
//! - A constraint that adds nothing, because the others already imply it, is
//!   refused too ([`SketchError::Redundant`]).
//! - [`Sketch::freedom`] reports how many degrees of freedom are left, overall
//!   and per entity, so the interface can show what is still free to move.
//!
//! The numerical core (solver, freedom analysis, conflict and redundancy
//! detection) is adapted from CADCraft (decision 0011, see `NOTICE`).
//!
//! Lengths are millimetres and angles are degrees at this interface.

mod equations;
mod profiles;
mod solver;
mod system;

pub use profiles::{Loop, LoopCurve, Profile, ProfileKey, Profiles};

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub use cndt_geom::Point2;
use serde::{Deserialize, Serialize};

/// Identifies an entity within one sketch. Never reused after removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub u32);

/// Identifies a constraint within one sketch. Never reused after removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConstraintId(pub u32);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "entity {}", self.0)
    }
}

impl fmt::Display for ConstraintId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "constraint {}", self.0)
    }
}

/// The geometry of a sketch entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    /// A point. The only geometry with coordinates of its own.
    Point {
        /// Position in sketch coordinates.
        at: Point2,
    },
    /// A straight line between two points.
    Line {
        /// The start point.
        start: EntityId,
        /// The end point.
        end: EntityId,
    },
    /// A full circle.
    Circle {
        /// The centre point.
        center: EntityId,
        /// Radius in millimetres.
        radius: f64,
    },
    /// A circular arc, counter-clockwise from `start` to `end`. Its radius is
    /// the distance from the centre to the start point, and the solver keeps
    /// the end point at the same distance.
    Arc {
        /// The centre point.
        center: EntityId,
        /// Where the arc starts.
        start: EntityId,
        /// Where the arc ends.
        end: EntityId,
    },
}

impl Geometry {
    /// A short name for messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Geometry::Point { .. } => "point",
            Geometry::Line { .. } => "line",
            Geometry::Circle { .. } => "circle",
            Geometry::Arc { .. } => "arc",
        }
    }

    /// The points this geometry is built on.
    fn points(&self) -> Vec<EntityId> {
        match *self {
            Geometry::Point { .. } => Vec::new(),
            Geometry::Line { start, end } => vec![start, end],
            Geometry::Circle { center, .. } => vec![center],
            Geometry::Arc { center, start, end } => vec![center, start, end],
        }
    }
}

/// One entity of a sketch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    /// Its geometry.
    pub geometry: Geometry,
    /// Construction geometry guides the sketch but is not part of the profile.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub construction: bool,
}

/// Which direction a distance is measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// The straight-line distance.
    Aligned,
    /// Along the sketch's X axis.
    Horizontal,
    /// Along the sketch's Y axis.
    Vertical,
}

/// A geometric or dimensional constraint. Dimensions are millimetres, angles
/// degrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Constraint {
    /// Two points in the same place.
    Coincident {
        /// A point.
        a: EntityId,
        /// Another point.
        b: EntityId,
    },
    /// A point on a line, or on the line's extension.
    PointOnLine {
        /// The point.
        point: EntityId,
        /// The line.
        line: EntityId,
    },
    /// A point on a circle or arc (or the arc's full circle).
    PointOnCurve {
        /// The point.
        point: EntityId,
        /// The circle or arc.
        curve: EntityId,
    },
    /// A point at the middle of a line.
    Midpoint {
        /// The point.
        point: EntityId,
        /// The line.
        line: EntityId,
    },
    /// A horizontal line.
    Horizontal {
        /// The line.
        line: EntityId,
    },
    /// A vertical line.
    Vertical {
        /// The line.
        line: EntityId,
    },
    /// Two points level with each other.
    HorizontalPoints {
        /// A point.
        a: EntityId,
        /// Another point.
        b: EntityId,
    },
    /// Two points one above the other.
    VerticalPoints {
        /// A point.
        a: EntityId,
        /// Another point.
        b: EntityId,
    },
    /// Two parallel lines.
    Parallel {
        /// A line.
        a: EntityId,
        /// Another line.
        b: EntityId,
    },
    /// Two lines at right angles.
    Perpendicular {
        /// A line.
        a: EntityId,
        /// Another line.
        b: EntityId,
    },
    /// Two lines on one infinite line.
    Collinear {
        /// A line.
        a: EntityId,
        /// Another line.
        b: EntityId,
    },
    /// Two lines of equal length, or two circles or arcs of equal radius.
    Equal {
        /// A line, circle or arc.
        a: EntityId,
        /// Another of the same kind.
        b: EntityId,
    },
    /// A line touching a circle or arc, or two circles or arcs touching.
    Tangent {
        /// A line, circle or arc.
        a: EntityId,
        /// A circle or arc.
        b: EntityId,
    },
    /// Two circles or arcs with one centre.
    Concentric {
        /// A circle or arc.
        a: EntityId,
        /// Another circle or arc.
        b: EntityId,
    },
    /// Two points mirrored about a line.
    Symmetric {
        /// A point.
        a: EntityId,
        /// Its mirror image.
        b: EntityId,
        /// The mirror line.
        axis: EntityId,
    },
    /// A point that stays where it is.
    Fix {
        /// The point.
        point: EntityId,
    },
    /// The distance between two points.
    Distance {
        /// A point.
        a: EntityId,
        /// Another point.
        b: EntityId,
        /// Which direction is measured.
        axis: Axis,
        /// Millimetres.
        value: f64,
    },
    /// The perpendicular distance from a point to a line.
    PointLineDistance {
        /// The point.
        point: EntityId,
        /// The line.
        line: EntityId,
        /// Millimetres.
        value: f64,
    },
    /// The length of a line.
    Length {
        /// The line.
        line: EntityId,
        /// Millimetres.
        value: f64,
    },
    /// The radius of a circle or arc.
    Radius {
        /// The circle or arc.
        curve: EntityId,
        /// Millimetres.
        value: f64,
    },
    /// The diameter of a circle or arc.
    Diameter {
        /// The circle or arc.
        curve: EntityId,
        /// Millimetres.
        value: f64,
    },
    /// The angle between two lines, measured as drawn.
    Angle {
        /// A line.
        a: EntityId,
        /// Another line.
        b: EntityId,
        /// Degrees, more than 0 and less than 180.
        value: f64,
    },
}

impl Constraint {
    /// A short name for messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Constraint::Coincident { .. } => "coincident",
            Constraint::PointOnLine { .. } => "point on line",
            Constraint::PointOnCurve { .. } => "point on curve",
            Constraint::Midpoint { .. } => "midpoint",
            Constraint::Horizontal { .. } | Constraint::HorizontalPoints { .. } => "horizontal",
            Constraint::Vertical { .. } | Constraint::VerticalPoints { .. } => "vertical",
            Constraint::Parallel { .. } => "parallel",
            Constraint::Perpendicular { .. } => "perpendicular",
            Constraint::Collinear { .. } => "collinear",
            Constraint::Equal { .. } => "equal",
            Constraint::Tangent { .. } => "tangent",
            Constraint::Concentric { .. } => "concentric",
            Constraint::Symmetric { .. } => "symmetric",
            Constraint::Fix { .. } => "fix",
            Constraint::Distance { .. } => "distance",
            Constraint::PointLineDistance { .. } => "point to line distance",
            Constraint::Length { .. } => "length",
            Constraint::Radius { .. } => "radius",
            Constraint::Diameter { .. } => "diameter",
            Constraint::Angle { .. } => "angle",
        }
    }

    /// The entities this constraint refers to.
    #[must_use]
    pub fn entities(&self) -> Vec<EntityId> {
        use Constraint as C;
        match *self {
            C::Fix { point } => vec![point],
            C::Horizontal { line } | C::Vertical { line } | C::Length { line, .. } => vec![line],
            C::Radius { curve, .. } | C::Diameter { curve, .. } => vec![curve],
            C::PointOnLine { point, line }
            | C::Midpoint { point, line }
            | C::PointLineDistance { point, line, .. } => vec![point, line],
            C::PointOnCurve { point, curve } => vec![point, curve],
            C::Symmetric { a, b, axis } => vec![a, b, axis],
            C::Coincident { a, b }
            | C::HorizontalPoints { a, b }
            | C::VerticalPoints { a, b }
            | C::Parallel { a, b }
            | C::Perpendicular { a, b }
            | C::Collinear { a, b }
            | C::Equal { a, b }
            | C::Tangent { a, b }
            | C::Concentric { a, b }
            | C::Distance { a, b, .. }
            | C::Angle { a, b, .. } => vec![a, b],
        }
    }

    /// The dimension's value, for dimensional constraints.
    #[must_use]
    pub fn value(&self) -> Option<f64> {
        use Constraint as C;
        match *self {
            C::Distance { value, .. }
            | C::PointLineDistance { value, .. }
            | C::Length { value, .. }
            | C::Radius { value, .. }
            | C::Diameter { value, .. }
            | C::Angle { value, .. } => Some(value),
            _ => None,
        }
    }

    fn value_mut(&mut self) -> Option<&mut f64> {
        use Constraint as C;
        match self {
            C::Distance { value, .. }
            | C::PointLineDistance { value, .. }
            | C::Length { value, .. }
            | C::Radius { value, .. }
            | C::Diameter { value, .. }
            | C::Angle { value, .. } => Some(value),
            _ => None,
        }
    }
}

/// Why a sketch change was refused. The sketch is unchanged whenever one of
/// these is returned.
#[derive(Debug, Clone, PartialEq)]
pub enum SketchError {
    /// The entity does not exist.
    UnknownEntity(EntityId),
    /// The constraint does not exist.
    UnknownConstraint(ConstraintId),
    /// A reference has the wrong kind of geometry.
    WrongGeometry {
        /// The entity referred to.
        entity: EntityId,
        /// What it is.
        found: &'static str,
        /// What was needed.
        expected: &'static str,
    },
    /// The same entity was given twice where two different ones are needed.
    SameEntity(EntityId),
    /// A dimension value outside what the constraint accepts.
    BadValue {
        /// The constraint kind.
        kind: &'static str,
        /// The value given.
        value: f64,
        /// What is accepted.
        expected: &'static str,
    },
    /// Other entities or constraints still use this entity.
    InUse {
        /// The entity that cannot be removed.
        entity: EntityId,
        /// The entities built on it.
        by: Vec<EntityId>,
    },
    /// A fixed point cannot be moved.
    Fixed(EntityId),
    /// The constraint has no value to change.
    NotADimension(ConstraintId),
    /// The constraints cannot all hold at once. When a new constraint caused
    /// it, that one is listed first, followed by the existing ones it fights.
    Conflict {
        /// The constraints involved; empty if they could not be narrowed down.
        constraints: Vec<ConstraintId>,
    },
    /// The constraint adds nothing: the listed constraints already imply it.
    Redundant {
        /// The constraints that already fix the same thing.
        constraints: Vec<ConstraintId>,
    },
    /// The constraint system is too large to solve interactively.
    TooLarge,
    /// A loaded sketch breaks the rules the editing methods keep.
    Corrupt(String),
}

fn ids_text(ids: &[ConstraintId]) -> String {
    if ids.is_empty() {
        return "other constraints".to_string();
    }
    ids.iter()
        .map(|id| id.0.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for SketchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SketchError::UnknownEntity(id) => write!(f, "{id} does not exist"),
            SketchError::UnknownConstraint(id) => write!(f, "{id} does not exist"),
            SketchError::WrongGeometry {
                entity,
                found,
                expected,
            } => {
                write!(f, "{entity} is a {found}, but this needs {expected}")
            }
            SketchError::SameEntity(id) => {
                write!(f, "{id} was picked twice; pick two different entities")
            }
            SketchError::BadValue {
                kind,
                value,
                expected,
            } => {
                write!(
                    f,
                    "a {kind} of {value} is not possible; it must be {expected}"
                )
            }
            SketchError::InUse { entity, by } => {
                let by: Vec<String> = by.iter().map(|id| id.0.to_string()).collect();
                write!(
                    f,
                    "{entity} is used by entities {}; remove those first",
                    by.join(", ")
                )
            }
            SketchError::Fixed(id) => {
                write!(f, "{id} is fixed; remove its fix constraint to move it")
            }
            SketchError::NotADimension(id) => write!(f, "{id} has no value to change"),
            SketchError::Conflict { constraints } => write!(
                f,
                "the constraints cannot all hold at once (conflicting: {})",
                ids_text(constraints)
            ),
            SketchError::Redundant { constraints } => write!(
                f,
                "this constraint is already implied by {}; it would over-constrain the sketch",
                ids_text(constraints)
            ),
            SketchError::Corrupt(reason) => write!(f, "the sketch data is damaged: {reason}"),
            SketchError::TooLarge => {
                write!(f, "the sketch has too many connected constraints to solve")
            }
        }
    }
}

impl std::error::Error for SketchError {}

/// How a solve should treat geometry the user is working on.
#[derive(Debug, Clone, Default)]
pub struct SolveOptions {
    /// Points that should move as little as possible, such as the ones the
    /// user just placed. Other geometry takes up the change.
    pub keep: BTreeSet<EntityId>,
    /// Points that must not move at all in this solve, as if fixed: the
    /// points the user just placed while drawing.
    pub hold: BTreeSet<EntityId>,
}

/// How constrained a sketch or an entity is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Some freedom left.
    Under,
    /// No freedom left and no redundant constraints.
    Fully,
    /// Some constraints duplicate others.
    Over,
    /// Too large to analyse.
    Unknown,
}

/// The degrees of freedom one entity has left.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityFreedom {
    /// The entity.
    pub id: EntityId,
    /// Independent ways it can still move; `None` if too large to analyse.
    pub dof: Option<usize>,
    /// Its state.
    pub status: Status,
}

/// What the constraints leave free in a sketch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Freedom {
    /// Remaining degrees of freedom of the whole sketch; `None` if too large
    /// to analyse.
    pub dof: Option<usize>,
    /// Equations that duplicate others.
    pub redundant: usize,
    /// The sketch's state.
    pub status: Status,
    /// Every entity, in id order.
    pub entities: Vec<EntityFreedom>,
}

/// A 2D sketch: entities and the constraints between them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sketch {
    entities: BTreeMap<EntityId, Entity>,
    constraints: BTreeMap<ConstraintId, Constraint>,
    next_entity: u32,
    next_constraint: u32,
}

impl Sketch {
    /// An empty sketch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The entities, in id order.
    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, e)| (*id, e))
    }

    /// The constraints, in id order.
    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintId, &Constraint)> {
        self.constraints.iter().map(|(id, c)| (*id, c))
    }

    /// One entity.
    #[must_use]
    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// One constraint.
    #[must_use]
    pub fn constraint(&self, id: ConstraintId) -> Option<&Constraint> {
        self.constraints.get(&id)
    }

    /// A point's position.
    #[must_use]
    pub fn point(&self, id: EntityId) -> Option<Point2> {
        match self.entities.get(&id)?.geometry {
            Geometry::Point { at } => Some(at),
            _ => None,
        }
    }

    /// The radius of a circle or arc.
    #[must_use]
    pub fn radius(&self, id: EntityId) -> Option<f64> {
        match self.entities.get(&id)?.geometry {
            Geometry::Circle { radius, .. } => Some(radius),
            Geometry::Arc { center, start, .. } => {
                Some(self.point(center)?.distance(self.point(start)?))
            }
            _ => None,
        }
    }

    /// Adds a point.
    pub fn add_point(&mut self, at: Point2) -> Result<EntityId, SketchError> {
        self.add(Geometry::Point { at }, false)
    }

    /// Adds a line between two existing points.
    pub fn add_line(&mut self, start: EntityId, end: EntityId) -> Result<EntityId, SketchError> {
        self.add(Geometry::Line { start, end }, false)
    }

    /// Adds a circle around an existing point.
    pub fn add_circle(&mut self, center: EntityId, radius: f64) -> Result<EntityId, SketchError> {
        self.add(Geometry::Circle { center, radius }, false)
    }

    /// Adds an arc, counter-clockwise from `start` to `end` around `center`.
    pub fn add_arc(
        &mut self,
        center: EntityId,
        start: EntityId,
        end: EntityId,
    ) -> Result<EntityId, SketchError> {
        self.add(Geometry::Arc { center, start, end }, false)
    }

    /// Adds any entity. Its points must already exist.
    pub fn add(&mut self, geometry: Geometry, construction: bool) -> Result<EntityId, SketchError> {
        let points = geometry.points();
        for &p in &points {
            self.expect_point(p)?;
        }
        let distinct: BTreeSet<EntityId> = points.iter().copied().collect();
        if distinct.len() != points.len() {
            let repeated = points
                .iter()
                .find(|p| points.iter().filter(|q| q == p).count() > 1)
                .copied()
                .unwrap_or(EntityId(0));
            return Err(SketchError::SameEntity(repeated));
        }
        match geometry {
            Geometry::Point { at } if !(at.x.is_finite() && at.y.is_finite()) => {
                return Err(SketchError::BadValue {
                    kind: "point coordinate",
                    value: if at.x.is_finite() { at.y } else { at.x },
                    expected: "a finite number",
                });
            }
            Geometry::Circle { radius, .. } if !(radius.is_finite() && radius > 0.0) => {
                return Err(SketchError::BadValue {
                    kind: "radius",
                    value: radius,
                    expected: "more than zero",
                });
            }
            _ => {}
        }
        let id = EntityId(self.next_entity);
        self.next_entity = self.next_entity.saturating_add(1);
        self.entities.insert(
            id,
            Entity {
                geometry,
                construction,
            },
        );
        Ok(id)
    }

    /// Removes an entity and every constraint on it. Refused while other
    /// entities are built on it (a line's end points, for example).
    pub fn remove_entity(&mut self, id: EntityId) -> Result<Vec<ConstraintId>, SketchError> {
        if !self.entities.contains_key(&id) {
            return Err(SketchError::UnknownEntity(id));
        }
        let by: Vec<EntityId> = self
            .entities
            .iter()
            .filter(|(_, e)| e.geometry.points().contains(&id))
            .map(|(other, _)| *other)
            .collect();
        if !by.is_empty() {
            return Err(SketchError::InUse { entity: id, by });
        }
        let removed: Vec<ConstraintId> = self
            .constraints
            .iter()
            .filter(|(_, c)| c.entities().contains(&id))
            .map(|(cid, _)| *cid)
            .collect();
        for cid in &removed {
            self.constraints.remove(cid);
        }
        self.entities.remove(&id);
        Ok(removed)
    }

    /// Adds a constraint and solves. Refused, leaving the sketch unchanged,
    /// if it is malformed, conflicts with the existing constraints, or adds
    /// nothing to them.
    pub fn add_constraint(
        &mut self,
        constraint: Constraint,
        options: &SolveOptions,
    ) -> Result<ConstraintId, SketchError> {
        self.check(&constraint)?;
        let id = ConstraintId(self.next_constraint);
        let mut trial = self.clone();
        trial.next_constraint = trial.next_constraint.saturating_add(1);
        trial.constraints.insert(id, constraint);
        system::add_checked(&mut trial, id, options)?;
        *self = trial;
        Ok(id)
    }

    /// Marks an entity as construction geometry or not, returning the
    /// previous setting.
    pub fn set_construction(
        &mut self,
        id: EntityId,
        construction: bool,
    ) -> Result<bool, SketchError> {
        let entity = self
            .entities
            .get_mut(&id)
            .ok_or(SketchError::UnknownEntity(id))?;
        Ok(std::mem::replace(&mut entity.construction, construction))
    }

    /// Checks every rule the editing methods keep. Used after loading a
    /// file, which bypasses them. Redundant constraints are allowed here;
    /// [`Sketch::freedom`] reports them.
    pub fn validate(&self) -> Result<(), SketchError> {
        for (&id, entity) in &self.entities {
            if id.0 >= self.next_entity {
                return Err(SketchError::Corrupt(format!(
                    "{id} is beyond the next free id"
                )));
            }
            let mut copy = self.clone();
            copy.entities.remove(&id);
            copy.next_entity = 0;
            copy.add(entity.geometry.clone(), entity.construction)?;
        }
        for (&id, constraint) in &self.constraints {
            if id.0 >= self.next_constraint {
                return Err(SketchError::Corrupt(format!(
                    "{id} is beyond the next free id"
                )));
            }
            self.check(constraint)?;
        }
        Ok(())
    }

    /// Removes a constraint. The geometry stays where it is.
    pub fn remove_constraint(&mut self, id: ConstraintId) -> Result<Constraint, SketchError> {
        self.constraints
            .remove(&id)
            .ok_or(SketchError::UnknownConstraint(id))
    }

    /// Changes a dimension's value and solves. Refused, leaving the sketch
    /// unchanged, if the new value cannot be reached.
    pub fn set_dimension(
        &mut self,
        id: ConstraintId,
        value: f64,
        options: &SolveOptions,
    ) -> Result<(), SketchError> {
        let mut trial = self.clone();
        let constraint = trial
            .constraints
            .get_mut(&id)
            .ok_or(SketchError::UnknownConstraint(id))?;
        *constraint
            .value_mut()
            .ok_or(SketchError::NotADimension(id))? = value;
        let constraint = constraint.clone();
        trial.check(&constraint)?;
        system::solve(&mut trial, options).map_err(|error| match error {
            SketchError::Conflict { mut constraints } => {
                constraints.retain(|c| *c != id);
                constraints.insert(0, id);
                SketchError::Conflict { constraints }
            }
            other => other,
        })?;
        *self = trial;
        Ok(())
    }

    /// Moves a point, then solves with that point held as still as the
    /// constraints allow: the drag of a sketch point.
    pub fn move_point(&mut self, id: EntityId, to: Point2) -> Result<(), SketchError> {
        self.expect_point(id)?;
        if self
            .constraints
            .values()
            .any(|c| *c == Constraint::Fix { point: id })
        {
            return Err(SketchError::Fixed(id));
        }
        let mut trial = self.clone();
        if let Some(Entity {
            geometry: Geometry::Point { at },
            ..
        }) = trial.entities.get_mut(&id)
        {
            *at = to;
        }
        let options = SolveOptions {
            keep: BTreeSet::from([id]),
            ..SolveOptions::default()
        };
        system::solve(&mut trial, &options)?;
        *self = trial;
        Ok(())
    }

    /// Solves every constraint. On failure nothing changes.
    pub fn solve(&mut self, options: &SolveOptions) -> Result<(), SketchError> {
        let mut trial = self.clone();
        system::solve(&mut trial, options)?;
        *self = trial;
        Ok(())
    }

    /// How many degrees of freedom the constraints leave, overall and per
    /// entity.
    #[must_use]
    pub fn freedom(&self) -> Freedom {
        system::freedom(self)
    }

    /// The geometry a reference must have.
    fn geometry(&self, id: EntityId) -> Result<&Geometry, SketchError> {
        self.entities
            .get(&id)
            .map(|e| &e.geometry)
            .ok_or(SketchError::UnknownEntity(id))
    }

    fn expect(
        &self,
        id: EntityId,
        expected: &'static str,
        ok: fn(&Geometry) -> bool,
    ) -> Result<(), SketchError> {
        let geometry = self.geometry(id)?;
        if ok(geometry) {
            Ok(())
        } else {
            Err(SketchError::WrongGeometry {
                entity: id,
                found: geometry.kind(),
                expected,
            })
        }
    }

    fn expect_point(&self, id: EntityId) -> Result<(), SketchError> {
        self.expect(id, "a point", |g| matches!(g, Geometry::Point { .. }))
    }

    fn expect_line(&self, id: EntityId) -> Result<(), SketchError> {
        self.expect(id, "a line", |g| matches!(g, Geometry::Line { .. }))
    }

    fn expect_curve(&self, id: EntityId) -> Result<(), SketchError> {
        self.expect(id, "a circle or arc", |g| {
            matches!(g, Geometry::Circle { .. } | Geometry::Arc { .. })
        })
    }

    /// Checks a constraint's references and value.
    fn check(&self, constraint: &Constraint) -> Result<(), SketchError> {
        use Constraint as C;
        let ids = constraint.entities();
        for (i, a) in ids.iter().enumerate() {
            if ids.iter().skip(i + 1).any(|b| b == a) {
                return Err(SketchError::SameEntity(*a));
            }
        }
        match *constraint {
            C::Coincident { a, b }
            | C::HorizontalPoints { a, b }
            | C::VerticalPoints { a, b }
            | C::Distance { a, b, .. } => {
                self.expect_point(a)?;
                self.expect_point(b)?;
            }
            C::Fix { point } => self.expect_point(point)?,
            C::PointOnLine { point, line }
            | C::Midpoint { point, line }
            | C::PointLineDistance { point, line, .. } => {
                self.expect_point(point)?;
                self.expect_line(line)?;
            }
            C::PointOnCurve { point, curve } => {
                self.expect_point(point)?;
                self.expect_curve(curve)?;
            }
            C::Horizontal { line } | C::Vertical { line } | C::Length { line, .. } => {
                self.expect_line(line)?;
            }
            C::Parallel { a, b }
            | C::Perpendicular { a, b }
            | C::Collinear { a, b }
            | C::Angle { a, b, .. } => {
                self.expect_line(a)?;
                self.expect_line(b)?;
            }
            C::Equal { a, b } => {
                if self.expect_line(a).is_ok() {
                    self.expect_line(b)?;
                } else {
                    self.expect_curve(a)
                        .map_err(|_| SketchError::WrongGeometry {
                            entity: a,
                            found: self.geometry(a).map_or("entity", Geometry::kind),
                            expected: "a line, circle or arc",
                        })?;
                    self.expect_curve(b)?;
                }
            }
            C::Tangent { a, b } => {
                if self.expect_line(a).is_err() {
                    self.expect_curve(a)
                        .map_err(|_| SketchError::WrongGeometry {
                            entity: a,
                            found: self.geometry(a).map_or("entity", Geometry::kind),
                            expected: "a line, circle or arc",
                        })?;
                }
                self.expect_curve(b)?;
            }
            C::Concentric { a, b } => {
                self.expect_curve(a)?;
                self.expect_curve(b)?;
            }
            C::Symmetric { a, b, axis } => {
                self.expect_point(a)?;
                self.expect_point(b)?;
                self.expect_line(axis)?;
            }
            C::Radius { curve, .. } | C::Diameter { curve, .. } => self.expect_curve(curve)?,
        }
        let bad = |expected| SketchError::BadValue {
            kind: constraint.kind(),
            value: constraint.value().unwrap_or(f64::NAN),
            expected,
        };
        match *constraint {
            C::Distance {
                axis: Axis::Aligned,
                value,
                ..
            }
            | C::Length { value, .. }
            | C::Radius { value, .. }
            | C::Diameter { value, .. }
                if !(value.is_finite() && value > 0.0) =>
            {
                Err(bad("more than zero (use coincident for zero)"))
            }
            C::Distance { value, .. } | C::PointLineDistance { value, .. }
                if !(value.is_finite() && value >= 0.0) =>
            {
                Err(bad("zero or more"))
            }
            C::Angle { value, .. } if !(value.is_finite() && value > 0.0 && value < 180.0) => {
                Err(bad("between 0 and 180 degrees (use parallel for 0)"))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
