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

/// Creates an empty sketch.
#[derive(Debug, Clone)]
pub struct CreateSketch {
    name: String,
    plane: Plane,
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
            created: None,
            undone: None,
        }
    }
}

impl Command for CreateSketch {
    fn name(&self) -> &'static str {
        "sketch.create"
    }

    fn label(&self) -> String {
        String::from("Create sketch")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        match (self.created, self.undone.take()) {
            (Some(id), Some(record)) => document.restore_sketch(id, record),
            _ => self.created = Some(document.add_sketch(&self.name, self.plane.clone())?),
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
    /// Adds a constraint.
    Constrain(Constraint),
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
            Self::Constrain(_) => "sketch.constrain",
            Self::SetDimension { .. } => "sketch.set_dimension",
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
            Self::Constrain(_) => "Add constraint",
            Self::SetDimension { .. } => "Change dimension",
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
            Self::Constrain(ref constraint) => {
                sketch.add_constraint(constraint.clone(), &options)?;
            }
            Self::SetDimension { constraint, value } => {
                sketch.set_dimension(constraint, value, &options)?;
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
    before: Option<Sketch>,
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
            .map(|record| record.sketch.clone())
            .ok_or(ModelError::UnknownSketch(self.sketch))?;
        document.edit_sketch(self.sketch, |sketch| self.edit.run(sketch))?;
        self.before = Some(before);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        document.replace_sketch(self.sketch, before)?;
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
}
