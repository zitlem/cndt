// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sketch commands.
//!
//! Edits inside a sketch are one [`EditSketch`] command each. It keeps a
//! copy of the sketch from before the edit, so undo restores it exactly, and
//! redo replays the edit on that same copy, which gives the same new ids.

use std::collections::BTreeSet;

use cndt_model::sketch::{Constraint, ConstraintId, EntityId, Geometry, Point2, SolveOptions};
use cndt_model::{Document, ModelError, Plane, Sketch, SketchError, SketchId, SketchRecord};

use crate::{Command, CommandError};

/// Creates a sketch: empty, or with imported curves.
#[derive(Debug, Clone)]
pub struct CreateSketch {
    name: String,
    plane: Plane,
    /// Curves to put in it, such as those read from a DXF file.
    content: Option<Sketch>,
    created: Option<SketchId>,
    /// The sketch as it was when undone, put back under the same id on redo.
    undone: Option<SketchRecord>,
}

impl CreateSketch {
    /// A command that adds a sketch on `plane`. An empty name gets `SketchN`.
    #[must_use]
    pub fn new(name: &str, plane: Plane) -> Self {
        Self {
            name: name.to_owned(),
            plane,
            content: None,
            created: None,
            undone: None,
        }
    }

    /// A command that adds a sketch on `plane` holding `content`.
    #[must_use]
    pub fn with_content(name: &str, plane: Plane, content: Sketch) -> Self {
        Self {
            content: Some(content),
            ..Self::new(name, plane)
        }
    }
}

/// A curve to import into a new sketch, in sketch coordinates (mm).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImportCurve {
    /// A straight line.
    Line {
        /// Start.
        start: [f64; 2],
        /// End.
        end: [f64; 2],
    },
    /// An arc, counter-clockwise from `start` to `end`.
    Arc {
        /// Centre.
        center: [f64; 2],
        /// Start.
        start: [f64; 2],
        /// End.
        end: [f64; 2],
    },
    /// A full circle.
    Circle {
        /// Centre.
        center: [f64; 2],
        /// Radius.
        radius: f64,
    },
}

/// The most curves one import may hold.
pub const MAX_IMPORT_CURVES: usize = 20_000;

/// Builds a sketch from curves. Ends within 0.0001 mm of each other become
/// one point, so a profile drawn as separate lines and arcs closes.
///
/// # Errors
///
/// A readable reason for no curves, too many, or a value that is not a
/// finite number or makes no curve.
pub fn sketch_from_curves(curves: &[ImportCurve]) -> Result<Sketch, String> {
    if curves.is_empty() {
        return Err(String::from("there are no curves to import"));
    }
    if curves.len() > MAX_IMPORT_CURVES {
        return Err(format!(
            "{} curves is more than the {MAX_IMPORT_CURVES} one sketch can take",
            curves.len()
        ));
    }
    let mut sketch = Sketch::new();
    let mut points: std::collections::BTreeMap<(i64, i64), EntityId> =
        std::collections::BTreeMap::new();
    let mut point = |sketch: &mut Sketch, p: [f64; 2]| -> Result<EntityId, String> {
        if !(p[0].is_finite() && p[1].is_finite()) || p[0].abs() > 1e7 || p[1].abs() > 1e7 {
            return Err(format!("{p:?} is not a usable point"));
        }
        #[allow(clippy::cast_possible_truncation)]
        let key = ((p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64);
        if let Some(id) = points.get(&key) {
            return Ok(*id);
        }
        let id = sketch
            .add_point(Point2 { x: p[0], y: p[1] })
            .map_err(|e| e.to_string())?;
        points.insert(key, id);
        Ok(id)
    };
    for curve in curves {
        match *curve {
            ImportCurve::Line { start, end } => {
                let (a, b) = (point(&mut sketch, start)?, point(&mut sketch, end)?);
                if a == b {
                    continue; // A line of no length.
                }
                sketch.add_line(a, b).map_err(|e| e.to_string())?;
            }
            ImportCurve::Circle { center, radius } => {
                if !(radius.is_finite() && radius > 1e-6) {
                    return Err(format!(
                        "a circle's radius must be more than zero, not {radius}"
                    ));
                }
                let c = point(&mut sketch, center)?;
                sketch.add_circle(c, radius).map_err(|e| e.to_string())?;
            }
            ImportCurve::Arc { center, start, end } => {
                let c = point(&mut sketch, center)?;
                let (s, e) = (point(&mut sketch, start)?, point(&mut sketch, end)?);
                if s == e || s == c {
                    continue;
                }
                sketch.add_arc(c, s, e).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(sketch)
}

impl Command for CreateSketch {
    fn name(&self) -> &'static str {
        if self.content.is_some() {
            "sketch.import"
        } else {
            "sketch.create"
        }
    }

    fn label(&self) -> String {
        String::from(if self.content.is_some() {
            "Import sketch"
        } else {
            "Create sketch"
        })
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        match (self.created, self.undone.take()) {
            (Some(id), Some(record)) => document.restore_sketch(id, record),
            _ => {
                let id = document.add_sketch(&self.name, self.plane.clone())?;
                if let Some(content) = &self.content {
                    document.replace_sketch(id, content.clone())?;
                }
                self.created = Some(id);
            }
        }
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let id = self.created.ok_or(CommandError::NotApplied)?;
        self.undone = Some(document.remove_sketch(id)?);
        Ok(())
    }
}

/// Deletes a sketch.
#[derive(Debug, Clone)]
pub struct DeleteSketch {
    id: SketchId,
    removed: Option<SketchRecord>,
}

impl DeleteSketch {
    /// A command that deletes sketch `id`.
    #[must_use]
    pub const fn new(id: SketchId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for DeleteSketch {
    fn name(&self) -> &'static str {
        "sketch.delete"
    }

    fn label(&self) -> String {
        String::from("Delete sketch")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        self.removed = Some(document.remove_sketch(self.id)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let record = self.removed.take().ok_or(CommandError::NotApplied)?;
        document.restore_sketch(self.id, record);
        Ok(())
    }
}

/// A point to draw from: one that exists, or a new one at a position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointRef {
    /// An existing point, as when the cursor snaps to it.
    Existing(EntityId),
    /// A new point.
    New(Point2),
}

/// One edit inside a sketch.
#[derive(Debug, Clone, PartialEq)]
pub enum SketchEdit {
    /// Adds a point.
    AddPoint(Point2),
    /// Adds a line, creating end points as needed.
    AddLine {
        /// Where it starts.
        start: PointRef,
        /// Where it ends.
        end: PointRef,
    },
    /// Adds a circle, creating its centre if needed.
    AddCircle {
        /// The centre.
        center: PointRef,
        /// Radius in millimetres.
        radius: f64,
    },
    /// Adds a counter-clockwise arc. Its end is brought onto the circle
    /// through its start.
    AddArc {
        /// The centre.
        center: PointRef,
        /// Where it starts; sets the radius.
        start: PointRef,
        /// Where it ends.
        end: PointRef,
    },
    /// Adds an axis-aligned rectangle from two opposite corners: four
    /// points, four lines, and horizontal and vertical constraints.
    AddRectangle {
        /// One corner.
        from: Point2,
        /// The opposite corner.
        to: Point2,
    },
    /// Adds a slot (a stadium): two arcs joined by two lines, from the
    /// centres of its two ends and its width. Tangent, equal-radius, radius
    /// and centre-distance constraints keep it a slot when it is edited; a
    /// construction line joins the two centres.
    AddSlot {
        /// The centre of one end.
        from: Point2,
        /// The centre of the other end.
        to: Point2,
        /// The width across the slot, in millimetres.
        width: f64,
    },
    /// Trims a line, arc or circle: removes the stretch of it that holds
    /// `at`, between the places where other curves cross it.
    Trim {
        /// The line, arc or circle to trim.
        entity: EntityId,
        /// Where on it to cut; the nearest stretch to this point goes.
        at: Point2,
    },
    /// Offsets a chain of lines and arcs, or a circle, a distance to the side
    /// that holds `toward`. The copies are new geometry.
    Offset {
        /// The curves: one circle, or lines and arcs joined end to end.
        entities: Vec<EntityId>,
        /// How far, in millimetres; more than zero.
        distance: f64,
        /// A point on the side to offset toward.
        toward: Point2,
    },
    /// Adds a constraint.
    Constrain(Constraint),
    /// Drives a dimension by an expression of the parameters, such as
    /// `thickness / 2`.
    SetDimensionExpression {
        /// The dimension.
        constraint: ConstraintId,
        /// The expression.
        expression: String,
    },
    /// Changes a dimension's value.
    SetDimension {
        /// The dimension.
        constraint: ConstraintId,
        /// The new value: millimetres, or degrees for an angle.
        value: f64,
    },
    /// Drags a point.
    MovePoint {
        /// The point.
        point: EntityId,
        /// Where to.
        to: Point2,
    },
    /// Marks an entity as construction geometry or not.
    SetConstruction {
        /// The entity.
        entity: EntityId,
        /// Construction or not.
        construction: bool,
    },
    /// Removes an entity and its constraints, and any lines, circles and
    /// arcs built on it if it is a point.
    RemoveEntity(EntityId),
    /// Removes a constraint.
    RemoveConstraint(ConstraintId),
}

impl SketchEdit {
    /// The registered command name for this edit.
    #[must_use]
    pub const fn command_name(&self) -> &'static str {
        match self {
            Self::AddPoint(_) => "sketch.add_point",
            Self::AddLine { .. } => "sketch.add_line",
            Self::AddCircle { .. } => "sketch.add_circle",
            Self::AddArc { .. } => "sketch.add_arc",
            Self::AddRectangle { .. } => "sketch.add_rectangle",
            Self::AddSlot { .. } => "sketch.add_slot",
            Self::Trim { .. } => "sketch.trim",
            Self::Offset { .. } => "sketch.offset",
            Self::Constrain(_) => "sketch.constrain",
            Self::SetDimension { .. } | Self::SetDimensionExpression { .. } => {
                "sketch.set_dimension"
            }
            Self::MovePoint { .. } => "sketch.move_point",
            Self::SetConstruction { .. } => "sketch.set_construction",
            Self::RemoveEntity(_) => "sketch.remove_entity",
            Self::RemoveConstraint(_) => "sketch.remove_constraint",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::AddPoint(_) => "Add point",
            Self::AddLine { .. } => "Add line",
            Self::AddCircle { .. } => "Add circle",
            Self::AddArc { .. } => "Add arc",
            Self::AddRectangle { .. } => "Add rectangle",
            Self::AddSlot { .. } => "Add slot",
            Self::Trim { .. } => "Trim",
            Self::Offset { .. } => "Offset",
            Self::Constrain(_) => "Add constraint",
            Self::SetDimension { .. } | Self::SetDimensionExpression { .. } => "Change dimension",
            Self::MovePoint { .. } => "Move point",
            Self::SetConstruction { .. } => "Toggle construction",
            Self::RemoveEntity(_) => "Delete sketch entity",
            Self::RemoveConstraint(_) => "Delete constraint",
        }
    }

    fn run(&self, sketch: &mut Sketch) -> Result<(), SketchError> {
        let options = SolveOptions::default();
        match *self {
            Self::AddPoint(at) => {
                sketch.add_point(at)?;
            }
            Self::AddLine { start, end } => {
                let start = point(sketch, start)?;
                let end = point(sketch, end)?;
                sketch.add_line(start, end)?;
            }
            Self::AddCircle { center, radius } => {
                let center = point(sketch, center)?;
                sketch.add_circle(center, radius)?;
            }
            Self::AddArc { center, start, end } => {
                let (center, start) = (point(sketch, center)?, point(sketch, start)?);
                let end = point(sketch, end)?;
                sketch.add_arc(center, start, end)?;
                // Bring the end onto the circle, holding what was placed.
                let hold = BTreeSet::from([center, start]);
                sketch.solve(&SolveOptions {
                    hold,
                    ..SolveOptions::default()
                })?;
            }
            Self::AddRectangle { from, to } => {
                for size in [to.x - from.x, to.y - from.y] {
                    if size.is_nan() || size.abs() <= 1e-9 {
                        return Err(SketchError::BadValue {
                            kind: "rectangle side",
                            value: size.abs(),
                            expected: "more than zero",
                        });
                    }
                }
                let corners = [
                    sketch.add_point(from)?,
                    sketch.add_point(Point2::new(to.x, from.y))?,
                    sketch.add_point(to)?,
                    sketch.add_point(Point2::new(from.x, to.y))?,
                ];
                let mut lines = Vec::with_capacity(4);
                for (i, &a) in corners.iter().enumerate() {
                    let b = corners.get((i + 1) % 4).copied().unwrap_or(a);
                    lines.push(sketch.add_line(a, b)?);
                }
                for (i, &line) in lines.iter().enumerate() {
                    let constraint = if i % 2 == 0 {
                        Constraint::Horizontal { line }
                    } else {
                        Constraint::Vertical { line }
                    };
                    sketch.add_constraint(constraint, &options)?;
                }
            }
            Self::AddSlot { from, to, width } => add_slot(sketch, from, to, width, &options)?,
            Self::Trim { entity, at } => crate::sketch_trim::trim(sketch, entity, at, &options)?,
            Self::Offset {
                ref entities,
                distance,
                toward,
            } => crate::sketch_offset::offset(sketch, entities, distance, toward)?,
            Self::Constrain(ref constraint) => {
                sketch.add_constraint(constraint.clone(), &options)?;
            }
            Self::SetDimension { constraint, value } => {
                sketch.set_dimension(constraint, value, &options)?;
            }
            // Needs the document's parameters; EditSketch applies it.
            Self::SetDimensionExpression { constraint, .. } => {
                return Err(SketchError::NotADimension(constraint));
            }
            Self::MovePoint { point, to } => sketch.move_point(point, to)?,
            Self::SetConstruction {
                entity,
                construction,
            } => {
                sketch.set_construction(entity, construction)?;
            }
            Self::RemoveEntity(entity) => remove_with_dependents(sketch, entity)?,
            Self::RemoveConstraint(constraint) => {
                sketch.remove_constraint(constraint)?;
            }
        }
        Ok(())
    }
}

/// Draws a slot between two end centres. The ends are counter-clockwise
/// arcs, so the profile detector sees one closed loop.
fn add_slot(
    sketch: &mut Sketch,
    from: Point2,
    to: Point2,
    width: f64,
    options: &SolveOptions,
) -> Result<(), SketchError> {
    let length = from.distance(to);
    if length.is_nan() || length <= 1e-9 {
        return Err(SketchError::BadValue {
            kind: "slot length",
            value: length,
            expected: "more than zero",
        });
    }
    if width.is_nan() || width <= 1e-9 {
        return Err(SketchError::BadValue {
            kind: "slot width",
            value: width,
            expected: "more than zero",
        });
    }
    let radius = width / 2.0;
    // Unit direction, and the unit normal to its left.
    let (dx, dy) = ((to.x - from.x) / length, (to.y - from.y) / length);
    let (nx, ny) = (-dy * radius, dx * radius);
    let offset = |c: Point2, sign: f64| Point2::new(c.x + sign * nx, c.y + sign * ny);
    let c1 = sketch.add_point(from)?;
    let c2 = sketch.add_point(to)?;
    let left1 = sketch.add_point(offset(from, 1.0))?;
    let left2 = sketch.add_point(offset(to, 1.0))?;
    let right1 = sketch.add_point(offset(from, -1.0))?;
    let right2 = sketch.add_point(offset(to, -1.0))?;
    let top = sketch.add_line(left1, left2)?;
    let bottom = sketch.add_line(right1, right2)?;
    // Counter-clockwise: the far end from its right side round to its left,
    // the near end from its left side round to its right.
    let far = sketch.add_arc(c2, right2, left2)?;
    let near = sketch.add_arc(c1, left1, right1)?;
    let axis = sketch.add_line(c1, c2)?;
    sketch.set_construction(axis, true)?;
    for (line, arc) in [(top, near), (top, far), (bottom, near), (bottom, far)] {
        sketch.add_constraint(Constraint::Tangent { a: line, b: arc }, options)?;
    }
    sketch.add_constraint(Constraint::Equal { a: near, b: far }, options)?;
    sketch.add_constraint(
        Constraint::Radius {
            curve: near,
            value: radius,
        },
        options,
    )?;
    sketch.add_constraint(
        Constraint::Length {
            line: axis,
            value: length,
        },
        options,
    )?;
    Ok(())
}

fn point(sketch: &mut Sketch, reference: PointRef) -> Result<EntityId, SketchError> {
    match reference {
        PointRef::Existing(id) => match sketch.entity(id).map(|e| &e.geometry) {
            Some(Geometry::Point { .. }) => Ok(id),
            Some(other) => Err(SketchError::WrongGeometry {
                entity: id,
                found: other.kind(),
                expected: "a point",
            }),
            None => Err(SketchError::UnknownEntity(id)),
        },
        PointRef::New(at) => sketch.add_point(at),
    }
}

/// Deletes an entity. Deleting a point also deletes what is built on it, as
/// deleting a corner does in Fusion 360. Deleting a line, circle or arc
/// also deletes its points once nothing else uses them.
fn remove_with_dependents(sketch: &mut Sketch, id: EntityId) -> Result<(), SketchError> {
    let uses = |sketch: &Sketch, point: EntityId| -> Vec<EntityId> {
        sketch
            .entities()
            .filter(|(_, e)| match e.geometry {
                Geometry::Line { start, end } => start == point || end == point,
                Geometry::Circle { center, .. } => center == point,
                Geometry::Arc { center, start, end } => {
                    center == point || start == point || end == point
                }
                Geometry::Point { .. } => false,
            })
            .map(|(other, _)| other)
            .collect()
    };
    let points: Vec<EntityId> = match sketch.entity(id).map(|e| &e.geometry) {
        None => return Err(SketchError::UnknownEntity(id)),
        Some(Geometry::Point { .. }) => {
            for user in uses(sketch, id) {
                sketch.remove_entity(user)?;
            }
            Vec::new()
        }
        Some(Geometry::Line { start, end }) => vec![*start, *end],
        Some(Geometry::Circle { center, .. }) => vec![*center],
        Some(Geometry::Arc { center, start, end }) => vec![*center, *start, *end],
    };
    sketch.remove_entity(id)?;
    for p in points {
        let constrained = sketch.constraints().any(|(_, c)| c.entities().contains(&p));
        if uses(sketch, p).is_empty() && !constrained {
            sketch.remove_entity(p)?;
        }
    }
    Ok(())
}

/// One edit inside a sketch, undoable.
#[derive(Debug, Clone)]
pub struct EditSketch {
    sketch: SketchId,
    edit: SketchEdit,
    before: Option<SketchRecord>,
}

impl EditSketch {
    /// A command that applies `edit` to sketch `sketch`.
    #[must_use]
    pub const fn new(sketch: SketchId, edit: SketchEdit) -> Self {
        Self {
            sketch,
            edit,
            before: None,
        }
    }
}

impl Command for EditSketch {
    fn name(&self) -> &'static str {
        self.edit.command_name()
    }

    fn label(&self) -> String {
        self.edit.label().to_owned()
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document
            .sketch(self.sketch)
            .cloned()
            .ok_or(ModelError::UnknownSketch(self.sketch))?;
        match &self.edit {
            SketchEdit::SetDimensionExpression {
                constraint,
                expression,
            } => {
                document.set_dimension_expression(self.sketch, *constraint, Some(expression))?;
            }
            SketchEdit::SetDimension { constraint, .. } => {
                document.edit_sketch(self.sketch, |sketch| self.edit.run(sketch))?;
                // A typed number replaces the expression that drove it.
                if before.expressions.contains_key(constraint)
                    && let Some(mut record) = document.sketch(self.sketch).cloned()
                {
                    record.expressions.remove(constraint);
                    document.set_sketch_record(self.sketch, record);
                }
            }
            _ => document.edit_sketch(self.sketch, |sketch| self.edit.run(sketch))?,
        }
        self.before = Some(before);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        document.set_sketch_record(self.sketch, before);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::Session;

    fn session_with_sketch() -> (Session, SketchId) {
        let mut session = Session::new(Document::default());
        session
            .execute(Box::new(CreateSketch::new("", Plane::Xy)))
            .unwrap();
        (session, SketchId(0))
    }

    fn edit(session: &mut Session, sketch: SketchId, edit: SketchEdit) -> Result<(), CommandError> {
        session.execute(Box::new(EditSketch::new(sketch, edit)))
    }

    fn sketch(session: &Session, id: SketchId) -> &Sketch {
        &session.document().sketch(id).unwrap().sketch
    }

    fn slot(from: (f64, f64), to: (f64, f64), width: f64) -> SketchEdit {
        SketchEdit::AddSlot {
            from: Point2::new(from.0, from.1),
            to: Point2::new(to.0, to.1),
            width,
        }
    }

    fn slot_area(length: f64, width: f64) -> f64 {
        length * width + std::f64::consts::PI * (width / 2.0).powi(2)
    }

    #[test]
    fn a_slot_is_one_closed_region_of_the_right_size() {
        // A slanted slot, so nothing depends on being axis aligned.
        let (mut session, id) = session_with_sketch();
        edit(&mut session, id, slot((0.0, 0.0), (24.0, 18.0), 6.0)).unwrap();
        let profiles = sketch(&session, id).profiles();
        assert_eq!(profiles.profiles.len(), 1);
        assert!(profiles.open.is_empty());
        let profile = &profiles.profiles[0];
        assert_eq!(profile.outer.curves.len(), 4);
        assert!(profile.holes.is_empty());
        let expected = slot_area(30.0, 6.0);
        assert!(
            (profile.outer.area.abs() - expected).abs() < 0.2,
            "area {} against {expected}",
            profile.outer.area
        );
        // Placement is the only freedom left: two for position, one for angle.
        assert_eq!(sketch(&session, id).freedom().dof, Some(3));
    }

    #[test]
    fn slot_dimensions_drive_it_and_it_stays_closed() {
        let (mut session, id) = session_with_sketch();
        edit(&mut session, id, slot((0.0, 0.0), (20.0, 0.0), 6.0)).unwrap();
        let dimensions: Vec<_> = sketch(&session, id)
            .constraints()
            .filter(|(_, c)| matches!(c, Constraint::Radius { .. } | Constraint::Length { .. }))
            .map(|(constraint, c)| (constraint, c.kind()))
            .collect();
        assert_eq!(dimensions.len(), 2);
        for (constraint, kind) in dimensions {
            // Width 10 is a radius of 5; the centres move to 30 apart.
            let value = if kind == "radius" { 5.0 } else { 30.0 };
            edit(
                &mut session,
                id,
                SketchEdit::SetDimension { constraint, value },
            )
            .unwrap();
        }
        let profiles = sketch(&session, id).profiles();
        assert_eq!(profiles.profiles.len(), 1);
        let area = profiles.profiles[0].outer.area.abs();
        assert!(
            (area - slot_area(30.0, 10.0)).abs() < 0.3,
            "area {area} against {}",
            slot_area(30.0, 10.0)
        );
    }

    #[test]
    fn slot_undo_and_redo() {
        let (mut session, id) = session_with_sketch();
        edit(&mut session, id, slot((0.0, 0.0), (20.0, 0.0), 6.0)).unwrap();
        let drawn = sketch(&session, id).clone();
        assert_eq!(session.undo_label().as_deref(), Some("Add slot"));
        session.undo().unwrap();
        assert_eq!(sketch(&session, id).entities().count(), 0);
        session.redo().unwrap();
        assert_eq!(sketch(&session, id), &drawn);
    }

    #[test]
    fn a_slot_with_no_length_or_no_width_is_refused_and_changes_nothing() {
        let (mut session, id) = session_with_sketch();
        for bad in [
            slot((5.0, 5.0), (5.0, 5.0), 6.0),
            slot((0.0, 0.0), (20.0, 0.0), 0.0),
            slot((0.0, 0.0), (20.0, 0.0), -3.0),
            slot((0.0, 0.0), (20.0, 0.0), f64::NAN),
        ] {
            let error = edit(&mut session, id, bad).unwrap_err().to_string();
            assert!(error.contains("more than zero"), "{error}");
            assert_eq!(sketch(&session, id).entities().count(), 0);
        }
    }

    fn line(session: &mut Session, id: SketchId, from: (f64, f64), to: (f64, f64)) -> EntityId {
        edit(
            session,
            id,
            SketchEdit::AddLine {
                start: PointRef::New(Point2::new(from.0, from.1)),
                end: PointRef::New(Point2::new(to.0, to.1)),
            },
        )
        .unwrap();
        sketch(session, id)
            .entities()
            .filter(|(_, e)| matches!(e.geometry, Geometry::Line { .. }))
            .map(|(entity, _)| entity)
            .max()
            .unwrap()
    }

    fn circle_about_origin(session: &mut Session, id: SketchId, radius: f64) -> EntityId {
        edit(
            session,
            id,
            SketchEdit::AddCircle {
                center: PointRef::New(Point2::new(0.0, 0.0)),
                radius,
            },
        )
        .unwrap();
        sketch(session, id)
            .entities()
            .find(|(_, e)| matches!(e.geometry, Geometry::Circle { .. }))
            .map(|(entity, _)| entity)
            .unwrap()
    }

    fn trim_at(session: &mut Session, id: SketchId, entity: EntityId, at: (f64, f64)) {
        try_trim(session, id, entity, at).unwrap();
    }

    fn try_trim(
        session: &mut Session,
        id: SketchId,
        entity: EntityId,
        at: (f64, f64),
    ) -> Result<(), CommandError> {
        edit(
            session,
            id,
            SketchEdit::Trim {
                entity,
                at: Point2::new(at.0, at.1),
            },
        )
    }

    /// Every line as its (smallest x, largest x), in order.
    fn line_spans(session: &Session, id: SketchId) -> Vec<(f64, f64)> {
        let s = sketch(session, id);
        let mut spans: Vec<(f64, f64)> = s
            .entities()
            .filter_map(|(_, e)| match e.geometry {
                Geometry::Line { start, end } => {
                    let (a, b) = (s.point(start)?, s.point(end)?);
                    Some((a.x.min(b.x), a.x.max(b.x)))
                }
                _ => None,
            })
            .collect();
        spans.sort_by(|a, b| a.partial_cmp(b).unwrap());
        spans
    }

    /// Every arc as its (start, end) points.
    fn arc_ends(session: &Session, id: SketchId) -> Vec<(Point2, Point2)> {
        let s = sketch(session, id);
        s.entities()
            .filter_map(|(_, e)| match e.geometry {
                Geometry::Arc { start, end, .. } => Some((s.point(start)?, s.point(end)?)),
                _ => None,
            })
            .collect()
    }

    fn near(p: Point2, x: f64, y: f64) -> bool {
        (p.x - x).abs() < 1e-6 && (p.y - y).abs() < 1e-6
    }

    #[test]
    fn trimming_a_line_between_two_crossings_leaves_its_two_ends() {
        let (mut session, id) = session_with_sketch();
        let bar = line(&mut session, id, (0.0, 0.0), (30.0, 0.0));
        line(&mut session, id, (10.0, -5.0), (10.0, 5.0));
        line(&mut session, id, (20.0, -5.0), (20.0, 5.0));
        trim_at(&mut session, id, bar, (15.0, 0.0));
        // The two uprights, and the bar cut back to 0..10 and 20..30.
        let spans = line_spans(&session, id);
        let expected = [(0.0, 10.0), (10.0, 10.0), (20.0, 20.0), (20.0, 30.0)];
        assert_eq!(spans.len(), expected.len(), "{spans:?}");
        for (want, got) in expected.iter().zip(&spans) {
            assert!(
                (want.0 - got.0).abs() < 1e-6 && (want.1 - got.1).abs() < 1e-6,
                "{spans:?}"
            );
        }
        // Each new end point is held on the line that cut it.
        let held = sketch(&session, id)
            .constraints()
            .filter(|(_, c)| matches!(c, Constraint::PointOnLine { .. }))
            .count();
        assert_eq!(held, 2);
    }

    #[test]
    fn trimming_the_end_of_a_line_keeps_the_rest() {
        let (mut session, id) = session_with_sketch();
        let bar = line(&mut session, id, (0.0, 0.0), (30.0, 0.0));
        line(&mut session, id, (10.0, -5.0), (10.0, 5.0));
        trim_at(&mut session, id, bar, (25.0, 0.0));
        let spans = line_spans(&session, id);
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert!(spans[0].0.abs() < 1e-6 && (spans[0].1 - 10.0).abs() < 1e-6);
        // The bar's far end point went with the cut-off part.
        let points = sketch(&session, id)
            .entities()
            .filter(|(_, e)| matches!(e.geometry, Geometry::Point { .. }))
            .count();
        assert_eq!(points, 4, "bar start, cut point, two upright ends");
    }

    #[test]
    fn a_crossing_at_a_corner_reuses_the_corner_point() {
        let (mut session, id) = session_with_sketch();
        // A bar, and an upright that starts on it at (10, 0).
        let bar = line(&mut session, id, (0.0, 0.0), (30.0, 0.0));
        let upright = line(&mut session, id, (10.0, 0.0), (10.0, 8.0));
        let before = sketch(&session, id).entities().count();
        trim_at(&mut session, id, bar, (20.0, 0.0));
        let s = sketch(&session, id);
        let Some(Geometry::Line { start, .. }) = s.entity(upright).map(|e| e.geometry.clone())
        else {
            panic!("the upright is gone");
        };
        let joined = s.entities().any(|(entity, e)| {
            entity != upright && matches!(e.geometry, Geometry::Line { end, .. } if end == start)
        });
        assert!(joined, "the bar should end on the upright's own point");
        // No point was made for the corner: the bar became a shorter line and
        // its far end point went.
        assert_eq!(s.entities().count(), before - 1);
    }

    #[test]
    fn trimming_a_circle_leaves_an_arc_and_trimming_an_arc_shortens_it() {
        let (mut session, id) = session_with_sketch();
        // A circle of radius 10, crossed by a vertical line at x = 6
        // (it meets the circle at y = 8 and y = -8).
        let circle = circle_about_origin(&mut session, id, 10.0);
        line(&mut session, id, (6.0, -12.0), (6.0, 12.0));
        // Remove the right-hand cap, x > 6.
        trim_at(&mut session, id, circle, (10.0, 0.0));
        assert!(sketch(&session, id).entity(circle).is_none());
        let arcs = arc_ends(&session, id);
        assert_eq!(arcs.len(), 1);
        // Counter-clockwise from the cut at the top round to the one at the bottom.
        assert!(near(arcs[0].0, 6.0, 8.0), "{:?}", arcs[0]);
        assert!(near(arcs[0].1, 6.0, -8.0), "{:?}", arcs[0]);

        // A horizontal line crosses the arc at (-10, 0): take the lower half.
        let arc = sketch(&session, id)
            .entities()
            .find(|(_, e)| matches!(e.geometry, Geometry::Arc { .. }))
            .map(|(entity, _)| entity)
            .unwrap();
        line(&mut session, id, (-12.0, 0.0), (-8.0, 0.0));
        trim_at(&mut session, id, arc, (-7.0, -7.0));
        let arcs = arc_ends(&session, id);
        assert_eq!(arcs.len(), 1, "{arcs:?}");
        assert!(near(arcs[0].0, 6.0, 8.0) && near(arcs[0].1, -10.0, 0.0));
    }

    #[test]
    fn trimming_with_nothing_crossing_is_refused_and_changes_nothing() {
        let (mut session, id) = session_with_sketch();
        let bar = line(&mut session, id, (0.0, 0.0), (30.0, 0.0));
        // One parallel to it, and one that only touches its end point.
        line(&mut session, id, (0.0, 5.0), (30.0, 5.0));
        line(&mut session, id, (30.0, 0.0), (30.0, 9.0));
        let drawn = sketch(&session, id).clone();
        let error = try_trim(&mut session, id, bar, (5.0, 0.0))
            .unwrap_err()
            .to_string();
        assert!(error.contains("nothing crosses"), "{error}");
        assert_eq!(sketch(&session, id), &drawn);
    }

    #[test]
    fn a_circle_crossed_once_a_point_and_a_missing_entity_are_refused() {
        let (mut session, id) = session_with_sketch();
        let circle = circle_about_origin(&mut session, id, 5.0);
        // A line that crosses the circle once: it ends inside.
        line(&mut session, id, (0.0, 0.0), (9.0, 0.0));
        let point = sketch(&session, id)
            .entities()
            .find(|(_, e)| matches!(e.geometry, Geometry::Point { .. }))
            .map(|(entity, _)| entity)
            .unwrap();
        for (entity, expected) in [
            (circle, "nothing crosses"),
            (point, "needs a line, arc or circle"),
            (EntityId(99), "does not exist"),
        ] {
            let error = try_trim(&mut session, id, entity, (1.0, 1.0))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn trim_undo_and_redo() {
        let (mut session, id) = session_with_sketch();
        let bar = line(&mut session, id, (0.0, 0.0), (30.0, 0.0));
        line(&mut session, id, (10.0, -5.0), (10.0, 5.0));
        let before = sketch(&session, id).clone();
        trim_at(&mut session, id, bar, (20.0, 0.0));
        let after = sketch(&session, id).clone();
        assert_ne!(before, after);
        assert_eq!(session.undo_label().as_deref(), Some("Trim"));
        session.undo().unwrap();
        assert_eq!(sketch(&session, id), &before);
        session.redo().unwrap();
        assert_eq!(sketch(&session, id), &after);
    }

    #[test]
    fn rectangle_then_dimensions_with_undo_and_redo() {
        let (mut session, id) = session_with_sketch();
        let rectangle = SketchEdit::AddRectangle {
            from: Point2::new(0.0, 0.0),
            to: Point2::new(90.0, 50.0),
        };
        edit(&mut session, id, rectangle).unwrap();
        assert_eq!(sketch(&session, id).entities().count(), 8);
        assert_eq!(sketch(&session, id).constraints().count(), 4);
        assert_eq!(sketch(&session, id).freedom().dof, Some(4));
        let width = SketchEdit::Constrain(Constraint::Length {
            line: EntityId(4),
            value: 96.0,
        });
        edit(&mut session, id, width).unwrap();
        let after_width = sketch(&session, id).clone();
        assert_eq!(session.undo_label().as_deref(), Some("Add constraint"));
        session.undo().unwrap();
        assert_eq!(sketch(&session, id).constraints().count(), 4);
        session.redo().unwrap();
        assert_eq!(
            sketch(&session, id),
            &after_width,
            "redo gives the same ids and geometry"
        );
    }

    #[test]
    fn a_refused_edit_changes_nothing_and_adds_no_undo_step() {
        let (mut session, id) = session_with_sketch();
        let a = PointRef::New(Point2::new(0.0, 0.0));
        edit(&mut session, id, SketchEdit::AddLine { start: a, end: a }).unwrap();
        let before = session.document().clone();
        let label = session.undo_label();
        let bad = SketchEdit::Constrain(Constraint::Horizontal { line: EntityId(0) });
        let error = edit(&mut session, id, bad).unwrap_err();
        assert!(error.to_string().contains("needs a line"), "{error}");
        assert_eq!(session.document(), &before);
        assert_eq!(session.undo_label(), label);
        let missing = edit(
            &mut session,
            SketchId(7),
            SketchEdit::AddPoint(Point2::new(0.0, 0.0)),
        );
        assert_eq!(
            missing,
            Err(CommandError::Model(ModelError::UnknownSketch(SketchId(7))))
        );
    }

    #[test]
    fn lines_snap_to_existing_points() {
        let (mut session, id) = session_with_sketch();
        let start = PointRef::New(Point2::new(0.0, 0.0));
        let corner = PointRef::New(Point2::new(10.0, 0.0));
        edit(&mut session, id, SketchEdit::AddLine { start, end: corner }).unwrap();
        let shared = PointRef::Existing(EntityId(1));
        let end = PointRef::New(Point2::new(10.0, 10.0));
        edit(&mut session, id, SketchEdit::AddLine { start: shared, end }).unwrap();
        // Three points, two lines: the corner is shared.
        assert_eq!(sketch(&session, id).entities().count(), 5);
    }

    #[test]
    fn arcs_end_on_their_circle() {
        let (mut session, id) = session_with_sketch();
        let arc = SketchEdit::AddArc {
            center: PointRef::New(Point2::new(0.0, 0.0)),
            start: PointRef::New(Point2::new(10.0, 0.0)),
            end: PointRef::New(Point2::new(0.0, 7.0)),
        };
        edit(&mut session, id, arc).unwrap();
        let s = sketch(&session, id);
        let end = s.point(EntityId(2)).unwrap();
        assert!((end.x.hypot(end.y) - 10.0).abs() < 1e-7, "{end:?}");
        assert!(
            s.point(EntityId(1)).unwrap().x.to_bits() == 10f64.to_bits(),
            "start kept"
        );
    }

    #[test]
    fn deleting_a_line_takes_its_unused_points() {
        let (mut session, id) = session_with_sketch();
        let rectangle = SketchEdit::AddRectangle {
            from: Point2::new(0.0, 0.0),
            to: Point2::new(9.0, 5.0),
        };
        edit(&mut session, id, rectangle).unwrap();
        // Deleting one corner point removes the two lines that meet there.
        edit(&mut session, id, SketchEdit::RemoveEntity(EntityId(0))).unwrap();
        let s = sketch(&session, id);
        assert_eq!(
            s.entities()
                .filter(|(_, e)| e.geometry.kind() == "line")
                .count(),
            2
        );
        // Deleting a free-standing line removes its points too.
        let mut session = session_with_sketch().0;
        let line = SketchEdit::AddLine {
            start: PointRef::New(Point2::new(0.0, 0.0)),
            end: PointRef::New(Point2::new(5.0, 0.0)),
        };
        edit(&mut session, id, line).unwrap();
        edit(&mut session, id, SketchEdit::RemoveEntity(EntityId(2))).unwrap();
        assert_eq!(sketch(&session, id).entities().count(), 0);
    }

    #[test]
    fn create_and_delete_sketches_undo() {
        let (mut session, id) = session_with_sketch();
        edit(
            &mut session,
            id,
            SketchEdit::AddPoint(Point2::new(1.0, 1.0)),
        )
        .unwrap();
        let with_point = session.document().clone();
        session.execute(Box::new(DeleteSketch::new(id))).unwrap();
        assert!(session.document().sketches().is_empty());
        session.undo().unwrap();
        assert_eq!(session.document(), &with_point);
        session.undo().unwrap();
        session.undo().unwrap();
        assert!(session.document().sketches().is_empty());
        // Redo brings it back under the same id.
        session.redo().unwrap();
        assert!(session.document().sketch(id).is_some());
    }

    #[test]
    fn imported_curves_share_their_ends() {
        // A square of four separate lines, one corner off by a hair, an arc
        // and a circle.
        let curves = vec![
            ImportCurve::Line {
                start: [0.0, 0.0],
                end: [10.0, 0.0],
            },
            ImportCurve::Line {
                start: [10.0, 0.0],
                end: [10.0, 10.0],
            },
            ImportCurve::Line {
                start: [10.0, 10.0],
                end: [0.0, 10.0],
            },
            ImportCurve::Line {
                start: [0.000_01, 10.0],
                end: [0.0, 0.0],
            },
            ImportCurve::Arc {
                center: [5.0, 5.0],
                start: [7.0, 5.0],
                end: [3.0, 5.0],
            },
            ImportCurve::Circle {
                center: [5.0, 5.0],
                radius: 1.0,
            },
        ];
        let sketch = sketch_from_curves(&curves).unwrap();
        let points = sketch
            .entities()
            .filter(|(_, e)| matches!(e.geometry, Geometry::Point { .. }))
            .count();
        // Four corners, the arc's centre (shared with the circle's) and its
        // two ends.
        assert_eq!(points, 7);
        assert_eq!(sketch.entities().count(), 7 + 6);
        assert!(sketch_from_curves(&[]).is_err());
        let bad = [ImportCurve::Circle {
            center: [0.0, 0.0],
            radius: -1.0,
        }];
        assert!(sketch_from_curves(&bad).unwrap_err().contains("radius"));
        // As a command: one undo removes the whole sketch.
        let mut session = Session::new(Document::default());
        session
            .execute(Box::new(CreateSketch::with_content(
                "Drawing",
                Plane::Xy,
                sketch,
            )))
            .unwrap();
        assert_eq!(session.document().sketches().len(), 1);
        session.undo().unwrap();
        assert!(session.document().sketches().is_empty());
    }
}
