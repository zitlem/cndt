// SPDX-License-Identifier: AGPL-3.0-or-later
//! Projecting a model edge into a sketch (decision 0033).

use cndt_model::sketch::{Constraint, EntityId, Point2, SolveOptions};
use cndt_model::{Document, ModelError, ProjectedShape, Projection, Sketch, SketchError, SketchId};

use crate::{Command, CommandError};

/// Adds the line, arc or circle an edge projects to, with its points fixed,
/// and remembers which edge it came from.
///
/// The shape is computed by whoever has the geometry kernel (the API) and
/// handed in; the engine itself has no kernel.
#[derive(Debug, Clone)]
pub struct ProjectEdge {
    sketch: SketchId,
    first: String,
    second: String,
    shape: ProjectedShape,
    before: Option<Box<Document>>,
}

impl ProjectEdge {
    /// A command that projects the edge between faces `first` and `second`,
    /// which makes `shape` in the plane of `sketch`.
    #[must_use]
    pub fn new(sketch: SketchId, first: &str, second: &str, shape: ProjectedShape) -> Self {
        Self {
            sketch,
            first: first.to_owned(),
            second: second.to_owned(),
            shape,
            before: None,
        }
    }
}

fn fixed_point(sketch: &mut Sketch, at: Point2) -> Result<EntityId, SketchError> {
    let point = sketch.add_point(at)?;
    sketch.add_constraint(Constraint::Fix { point }, &SolveOptions::default())?;
    Ok(point)
}

/// Draws the shape with every point fixed; returns the curve.
fn draw(sketch: &mut Sketch, shape: ProjectedShape) -> Result<EntityId, SketchError> {
    match shape {
        ProjectedShape::Line { start, end } => {
            let (a, b) = (fixed_point(sketch, start)?, fixed_point(sketch, end)?);
            sketch.add_line(a, b)
        }
        ProjectedShape::Circle { center, radius } => {
            let c = fixed_point(sketch, center)?;
            sketch.add_circle(c, radius)
        }
        ProjectedShape::Arc { center, start, end } => {
            // The kernel's outline is 32-bit; put the end exactly on the
            // circle the start defines, as the solver requires.
            let radius = center.distance(start);
            let reach = center.distance(end);
            let end = if reach > 0.0 {
                Point2::new(
                    center.x + (end.x - center.x) * radius / reach,
                    center.y + (end.y - center.y) * radius / reach,
                )
            } else {
                end
            };
            let c = fixed_point(sketch, center)?;
            let (a, b) = (fixed_point(sketch, start)?, fixed_point(sketch, end)?);
            sketch.add_arc(c, a, b)
        }
    }
}

impl Command for ProjectEdge {
    fn name(&self) -> &'static str {
        "sketch.project_edge"
    }

    fn label(&self) -> String {
        String::from("Project edge")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document.clone();
        let result = (|| -> Result<(), ModelError> {
            let shape = self.shape;
            let entity = document.edit_sketch(self.sketch, |sketch| draw(sketch, shape))?;
            document.add_projection(
                self.sketch,
                Projection {
                    first: self.first.clone(),
                    second: self.second.clone(),
                    entity,
                },
            )
        })();
        match result {
            Ok(()) => {
                self.before = Some(Box::new(before));
                Ok(())
            }
            Err(error) => {
                *document = before;
                Err(error.into())
            }
        }
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        *document = *before;
        Ok(())
    }
}
