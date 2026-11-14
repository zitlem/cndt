// SPDX-License-Identifier: AGPL-3.0-or-later
//! Timeline commands: add, change and delete solid features.

use cndt_model::sketch::{EntityId, Geometry, ProfileKey};
use cndt_model::{
    Document, EdgeTarget, Feature, FeatureEntry, FeatureId, HoleSpec, ModelError, Operation,
    SketchId, default_profiles,
};

use crate::{Command, CommandError};

/// Adds an extrusion at the end of the timeline.
#[derive(Debug, Clone)]
pub struct AddExtrude {
    sketch: SketchId,
    profiles: Option<Vec<ProfileKey>>,
    distance: f64,
    expression: Option<String>,
    operation: Operation,
    name: Option<String>,
    created: Option<FeatureId>,
    undone: Option<(usize, FeatureEntry)>,
}

impl AddExtrude {
    /// Extrudes `profiles` of `sketch`, or when `None`, every region that is
    /// material when the sketch is read as a part outline.
    #[must_use]
    pub const fn new(
        sketch: SketchId,
        profiles: Option<Vec<ProfileKey>>,
        distance: f64,
        operation: Operation,
        name: Option<String>,
    ) -> Self {
        Self {
            sketch,
            profiles,
            distance,
            expression: None,
            operation,
            name,
            created: None,
            undone: None,
        }
    }
}

impl AddExtrude {
    /// Makes the distance follow an expression of the parameters, such as
    /// `thickness`, instead of the plain number.
    #[must_use]
    pub fn with_expression(mut self, expression: Option<String>) -> Self {
        self.expression = expression;
        self
    }
}

impl Command for AddExtrude {
    fn name(&self) -> &'static str {
        "feature.extrude"
    }

    fn label(&self) -> String {
        String::from(match self.operation {
            Operation::Join => "Extrude",
            Operation::Cut => "Extrude cut",
        })
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        if let Some((index, entry)) = self.undone.take() {
            document.restore_feature(index, entry);
            return Ok(());
        }
        let record = document
            .sketch(self.sketch)
            .ok_or(ModelError::UnknownSketch(self.sketch))?;
        let profiles = match &self.profiles {
            Some(p) => p.clone(),
            None => default_profiles(&record.sketch),
        };
        if profiles.is_empty() {
            return Err(ModelError::InvalidFeature(format!(
                "{} has no closed profile to extrude; close the outline first",
                record.name
            ))
            .into());
        }
        let name = match &self.name {
            Some(name) => name.clone(),
            None => document.next_feature_name("Extrude"),
        };
        let distance = match &self.expression {
            Some(text) => document.evaluate(text)?,
            None => self.distance,
        };
        let feature = Feature::Extrude {
            name,
            sketch: self.sketch,
            profiles,
            distance,
            distance_expression: self.expression.clone(),
            operation: self.operation,
        };
        self.created = Some(document.add_feature(feature)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let id = self.created.ok_or(CommandError::NotApplied)?;
        self.undone = Some(document.remove_feature(id)?);
        Ok(())
    }
}

/// Changes an extrusion's values.
#[derive(Debug, Clone)]
pub struct EditExtrude {
    id: FeatureId,
    distance: Option<f64>,
    expression: Option<String>,
    operation: Option<Operation>,
    profiles: Option<Vec<ProfileKey>>,
    previous: Option<Feature>,
}

impl EditExtrude {
    /// Changes whichever of the values are given.
    #[must_use]
    pub const fn new(
        id: FeatureId,
        distance: Option<f64>,
        operation: Option<Operation>,
        profiles: Option<Vec<ProfileKey>>,
    ) -> Self {
        Self {
            id,
            distance,
            expression: None,
            operation,
            profiles,
            previous: None,
        }
    }
}

impl EditExtrude {
    /// Makes the distance follow an expression instead.
    #[must_use]
    pub fn with_expression(mut self, expression: Option<String>) -> Self {
        self.expression = expression;
        self
    }
}

impl Command for EditExtrude {
    fn name(&self) -> &'static str {
        "feature.edit_extrude"
    }

    fn label(&self) -> String {
        String::from("Edit extrude")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let mut feature = document
            .feature(self.id)
            .cloned()
            .ok_or(ModelError::UnknownFeature(self.id))?;
        let Feature::Extrude {
            distance,
            distance_expression,
            operation,
            profiles,
            ..
        } = &mut feature
        else {
            return Err(
                ModelError::InvalidFeature(String::from("that feature is not an extrude")).into(),
            );
        };
        // A plain number replaces an expression; an expression sets both.
        if let Some(text) = &self.expression {
            *distance = document.evaluate(text)?;
            *distance_expression = Some(text.clone());
        } else if let Some(d) = self.distance {
            *distance = d;
            *distance_expression = None;
        }
        if let Some(o) = self.operation {
            *operation = o;
        }
        if let Some(p) = &self.profiles {
            p.clone_into(profiles);
        }
        self.previous = Some(document.replace_feature(self.id, feature)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let previous = self.previous.take().ok_or(CommandError::NotApplied)?;
        document.replace_feature(self.id, previous)?;
        Ok(())
    }
}

/// The points a new hole uses when none are picked: the sketch's free-standing
/// points, the ones no line, circle or arc is drawn from.
fn standalone_points(document: &Document, sketch: SketchId) -> Vec<EntityId> {
    let Some(record) = document.sketch(sketch) else {
        return Vec::new();
    };
    let used: Vec<EntityId> = record
        .sketch
        .entities()
        .flat_map(|(_, e)| match e.geometry {
            Geometry::Line { start, end } => vec![start, end],
            Geometry::Circle { center, .. } => vec![center],
            Geometry::Arc { center, start, end } => vec![center, start, end],
            Geometry::Point { .. } => Vec::new(),
        })
        .collect();
    record
        .sketch
        .entities()
        .filter(|(id, e)| {
            matches!(e.geometry, Geometry::Point { .. }) && !e.construction && !used.contains(id)
        })
        .map(|(id, _)| id)
        .collect()
}

/// Adds a hole feature at the end of the timeline.
#[derive(Debug, Clone)]
pub struct AddHole {
    sketch: SketchId,
    points: Option<Vec<EntityId>>,
    spec: HoleSpec,
    name: Option<String>,
    created: Option<FeatureId>,
    undone: Option<(usize, FeatureEntry)>,
}

impl AddHole {
    /// Holes at `points` of `sketch`, or when `None`, at every free-standing
    /// point of it.
    #[must_use]
    pub const fn new(
        sketch: SketchId,
        points: Option<Vec<EntityId>>,
        spec: HoleSpec,
        name: Option<String>,
    ) -> Self {
        Self {
            sketch,
            points,
            spec,
            name,
            created: None,
            undone: None,
        }
    }
}

impl Command for AddHole {
    fn name(&self) -> &'static str {
        "feature.hole"
    }

    fn label(&self) -> String {
        String::from("Hole")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        if let Some((index, entry)) = self.undone.take() {
            document.restore_feature(index, entry);
            return Ok(());
        }
        let record = document
            .sketch(self.sketch)
            .ok_or(ModelError::UnknownSketch(self.sketch))?;
        let points = match &self.points {
            Some(p) => p.clone(),
            None => standalone_points(document, self.sketch),
        };
        if points.is_empty() {
            return Err(ModelError::InvalidFeature(format!(
                "{} has no points to put holes at; add points to the sketch first",
                record.name
            ))
            .into());
        }
        let name = match &self.name {
            Some(name) => name.clone(),
            None => document.next_feature_name("Hole"),
        };
        let feature = Feature::Hole {
            name,
            sketch: self.sketch,
            points,
            spec: self.spec.clone(),
        };
        self.created = Some(document.add_feature(feature)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let id = self.created.ok_or(CommandError::NotApplied)?;
        self.undone = Some(document.remove_feature(id)?);
        Ok(())
    }
}

/// Changes a hole's size and type, keeping its points.
#[derive(Debug, Clone)]
pub struct EditHole {
    id: FeatureId,
    spec: HoleSpec,
    previous: Option<Feature>,
}

impl EditHole {
    /// Gives hole feature `id` a new size and type.
    #[must_use]
    pub const fn new(id: FeatureId, spec: HoleSpec) -> Self {
        Self {
            id,
            spec,
            previous: None,
        }
    }
}

impl Command for EditHole {
    fn name(&self) -> &'static str {
        "feature.edit_hole"
    }

    fn label(&self) -> String {
        String::from("Edit hole")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let mut feature = document
            .feature(self.id)
            .cloned()
            .ok_or(ModelError::UnknownFeature(self.id))?;
        let Feature::Hole { spec, .. } = &mut feature else {
            return Err(
                ModelError::InvalidFeature(String::from("that feature is not a hole")).into(),
            );
        };
        *spec = self.spec.clone();
        self.previous = Some(document.replace_feature(self.id, feature)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let previous = self.previous.take().ok_or(CommandError::NotApplied)?;
        document.replace_feature(self.id, previous)?;
        Ok(())
    }
}

/// Chamfer or fillet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendKind {
    /// A 45° bevel.
    Chamfer,
    /// A round.
    Fillet,
}

/// Adds a chamfer or fillet at the end of the timeline.
#[derive(Debug, Clone)]
pub struct AddBlend {
    kind: BlendKind,
    edges: Vec<EdgeTarget>,
    size: f64,
    name: Option<String>,
    created: Option<FeatureId>,
    undone: Option<(usize, FeatureEntry)>,
}

impl AddBlend {
    /// Blends `edges` by `size` millimetres (chamfer distance or radius).
    #[must_use]
    pub const fn new(
        kind: BlendKind,
        edges: Vec<EdgeTarget>,
        size: f64,
        name: Option<String>,
    ) -> Self {
        Self {
            kind,
            edges,
            size,
            name,
            created: None,
            undone: None,
        }
    }
}

impl Command for AddBlend {
    fn name(&self) -> &'static str {
        match self.kind {
            BlendKind::Chamfer => "feature.chamfer",
            BlendKind::Fillet => "feature.fillet",
        }
    }

    fn label(&self) -> String {
        String::from(match self.kind {
            BlendKind::Chamfer => "Chamfer",
            BlendKind::Fillet => "Fillet",
        })
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        if let Some((index, entry)) = self.undone.take() {
            document.restore_feature(index, entry);
            return Ok(());
        }
        let prefix = self.label();
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| document.next_feature_name(&prefix));
        let edges = self.edges.clone();
        let feature = match self.kind {
            BlendKind::Chamfer => Feature::Chamfer {
                name,
                edges,
                distance: self.size,
            },
            BlendKind::Fillet => Feature::Fillet {
                name,
                edges,
                radius: self.size,
            },
        };
        self.created = Some(document.add_feature(feature)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let id = self.created.ok_or(CommandError::NotApplied)?;
        self.undone = Some(document.remove_feature(id)?);
        Ok(())
    }
}

/// Changes a chamfer's distance or a fillet's radius.
#[derive(Debug, Clone)]
pub struct EditBlend {
    id: FeatureId,
    size: f64,
    previous: Option<Feature>,
}

impl EditBlend {
    /// Gives blend feature `id` a new size.
    #[must_use]
    pub const fn new(id: FeatureId, size: f64) -> Self {
        Self {
            id,
            size,
            previous: None,
        }
    }
}

impl Command for EditBlend {
    fn name(&self) -> &'static str {
        "feature.edit_blend"
    }

    fn label(&self) -> String {
        String::from("Edit blend")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let mut feature = document
            .feature(self.id)
            .cloned()
            .ok_or(ModelError::UnknownFeature(self.id))?;
        match &mut feature {
            Feature::Chamfer { distance: size, .. } | Feature::Fillet { radius: size, .. } => {
                *size = self.size
            }
            _ => {
                return Err(ModelError::InvalidFeature(String::from(
                    "that feature is not a chamfer or fillet",
                ))
                .into());
            }
        }
        self.previous = Some(document.replace_feature(self.id, feature)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let previous = self.previous.take().ok_or(CommandError::NotApplied)?;
        document.replace_feature(self.id, previous)?;
        Ok(())
    }
}

/// A pattern's grid: copies and spacing along sketch X and Y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// Copies along X, counting the original.
    pub count_x: u32,
    /// Millimetres between them.
    pub spacing_x: f64,
    /// Copies along Y, counting the original.
    pub count_y: u32,
    /// Millimetres between them.
    pub spacing_y: f64,
}

/// Adds a pattern of an earlier feature, or with `id`, changes one's grid.
#[derive(Debug, Clone)]
pub struct SetPattern {
    id: Option<FeatureId>,
    source: FeatureId,
    grid: Grid,
    name: Option<String>,
    before: Option<Box<Document>>,
}

impl SetPattern {
    /// A new pattern repeating `source`.
    #[must_use]
    pub const fn add(source: FeatureId, grid: Grid, name: Option<String>) -> Self {
        Self {
            id: None,
            source,
            grid,
            name,
            before: None,
        }
    }

    /// A new grid for pattern `id`.
    #[must_use]
    pub const fn edit(id: FeatureId, grid: Grid) -> Self {
        Self {
            id: Some(id),
            source: FeatureId(0),
            grid,
            name: None,
            before: None,
        }
    }
}

impl Command for SetPattern {
    fn name(&self) -> &'static str {
        if self.id.is_some() {
            "feature.edit_pattern"
        } else {
            "feature.pattern"
        }
    }

    fn label(&self) -> String {
        String::from(if self.id.is_some() {
            "Edit pattern"
        } else {
            "Pattern"
        })
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document.clone();
        let Grid {
            count_x,
            spacing_x,
            count_y,
            spacing_y,
        } = self.grid;
        match self.id {
            Some(id) => {
                let Some(Feature::Pattern { name, feature, .. }) = document.feature(id).cloned()
                else {
                    return Err(ModelError::InvalidFeature(String::from(
                        "that feature is not a pattern",
                    ))
                    .into());
                };
                let pattern = Feature::Pattern {
                    name,
                    feature,
                    count_x,
                    spacing_x,
                    count_y,
                    spacing_y,
                };
                document.replace_feature(id, pattern)?;
            }
            None => {
                match document.feature(self.source) {
                    Some(Feature::Extrude { .. } | Feature::Hole { .. }) => {}
                    Some(_) => {
                        return Err(ModelError::InvalidFeature(String::from(
                            "only extrudes and holes can be patterned for now",
                        ))
                        .into());
                    }
                    None => return Err(ModelError::UnknownFeature(self.source).into()),
                }
                let name = self
                    .name
                    .clone()
                    .unwrap_or_else(|| document.next_feature_name("Pattern"));
                let pattern = Feature::Pattern {
                    name,
                    feature: self.source,
                    count_x,
                    spacing_x,
                    count_y,
                    spacing_y,
                };
                document.add_feature(pattern)?;
            }
        }
        self.before = Some(Box::new(before));
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        *document = *before;
        Ok(())
    }
}

/// Deletes a feature from the timeline.
#[derive(Debug, Clone)]
pub struct DeleteFeature {
    id: FeatureId,
    removed: Option<(usize, FeatureEntry)>,
}

impl DeleteFeature {
    /// Deletes feature `id`.
    #[must_use]
    pub const fn new(id: FeatureId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for DeleteFeature {
    fn name(&self) -> &'static str {
        "feature.delete"
    }

    fn label(&self) -> String {
        String::from("Delete feature")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        self.removed = Some(document.remove_feature(self.id)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let (index, entry) = self.removed.take().ok_or(CommandError::NotApplied)?;
        document.restore_feature(index, entry);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{EditSketch, Session, SketchEdit};
    use cndt_model::Plane;
    use cndt_model::sketch::Point2;

    fn with_rectangle() -> (Session, SketchId) {
        let mut document = Document::default();
        let id = document.add_sketch("", Plane::Xy).unwrap();
        let mut session = Session::new(document);
        let rectangle = SketchEdit::AddRectangle {
            from: Point2::new(0.0, 0.0),
            to: Point2::new(9.0, 5.0),
        };
        session
            .execute(Box::new(EditSketch::new(id, rectangle)))
            .unwrap();
        (session, id)
    }

    #[test]
    fn extrude_edit_delete_with_undo_and_redo() {
        let (mut session, sketch) = with_rectangle();
        session
            .execute(Box::new(AddExtrude::new(
                sketch,
                None,
                10.0,
                Operation::Join,
                None,
            )))
            .unwrap();
        let added = session.document().clone();
        let Feature::Extrude { name, profiles, .. } = &added.features()[0].feature else {
            panic!()
        };
        assert_eq!(name, "Extrude1");
        assert_eq!(profiles.len(), 1);
        session
            .execute(Box::new(EditExtrude::new(
                FeatureId(0),
                Some(25.0),
                None,
                None,
            )))
            .unwrap();
        session
            .execute(Box::new(DeleteFeature::new(FeatureId(0))))
            .unwrap();
        assert!(session.document().features().is_empty());
        session.undo().unwrap();
        session.undo().unwrap();
        assert_eq!(session.document(), &added);
        session.undo().unwrap();
        assert!(session.document().features().is_empty());
        session.redo().unwrap();
        assert_eq!(
            session.document(),
            &added,
            "redo restores the same id and name"
        );
    }

    #[test]
    fn holes_default_to_free_standing_points() {
        let (mut session, sketch) = with_rectangle();
        session
            .execute(Box::new(AddExtrude::new(
                sketch,
                None,
                10.0,
                Operation::Join,
                None,
            )))
            .unwrap();
        let spec = HoleSpec {
            diameter: 3.0,
            depth: cndt_model::HoleDepth::Through,
            tip_angle: 118.0,
            counterbore: None,
            thread: None,
            fit: None,
        };
        // The rectangle's corners are not free-standing: nothing to drill yet.
        let error = session
            .execute(Box::new(AddHole::new(sketch, None, spec.clone(), None)))
            .unwrap_err();
        assert!(error.to_string().contains("no points"), "{error}");
        let add = SketchEdit::AddPoint(Point2::new(4.5, 2.5));
        session
            .execute(Box::new(EditSketch::new(sketch, add)))
            .unwrap();
        session
            .execute(Box::new(AddHole::new(sketch, None, spec.clone(), None)))
            .unwrap();
        let Feature::Hole { name, points, .. } = &session.document().features()[1].feature else {
            panic!()
        };
        assert_eq!((name.as_str(), points.len()), ("Hole1", 1));
        let bigger = HoleSpec {
            diameter: 4.0,
            ..spec
        };
        session
            .execute(Box::new(EditHole::new(FeatureId(1), bigger)))
            .unwrap();
        session.undo().unwrap();
        let Feature::Hole { spec, .. } = &session.document().features()[1].feature else {
            panic!()
        };
        assert!((spec.diameter - 3.0).abs() < 1e-12);
    }

    #[test]
    fn an_open_sketch_cannot_be_extruded() {
        let mut document = Document::default();
        let id = document.add_sketch("", Plane::Xy).unwrap();
        let mut session = Session::new(document);
        let error = session
            .execute(Box::new(AddExtrude::new(
                id,
                None,
                10.0,
                Operation::Join,
                None,
            )))
            .unwrap_err();
        assert!(error.to_string().contains("no closed profile"), "{error}");
        let bad = EditExtrude::new(FeatureId(3), Some(1.0), None, None);
        assert!(session.execute(Box::new(bad)).is_err());
    }
}

/// Changes the CAM definitions (decision 0014). Undo restores the document
/// exactly.
#[derive(Debug, Clone)]
pub struct EditCam {
    name: &'static str,
    edit: cndt_model::cam::CamEdit,
    before: Option<Box<Document>>,
}

impl EditCam {
    /// A command registered as `name` that applies `edit`.
    #[must_use]
    pub const fn new(name: &'static str, edit: cndt_model::cam::CamEdit) -> Self {
        Self {
            name,
            edit,
            before: None,
        }
    }
}

impl Command for EditCam {
    fn name(&self) -> &'static str {
        self.name
    }

    fn label(&self) -> String {
        use cndt_model::cam::CamEdit as E;
        String::from(match self.edit {
            E::SetStock(_) => "Set stock",
            E::AddSetup { .. } => "Add setup",
            E::RemoveSetup(_) => "Delete setup",
            E::AddOperation { .. } => "Add operation",
            E::AddOperations { .. } => "Add operations",
            E::RemoveOperation { .. } => "Delete operation",
            E::SetTool { .. } => "Set tool",
            E::RemoveTool(_) => "Delete tool",
        })
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document.clone();
        document.edit_cam(self.edit.clone())?;
        self.before = Some(Box::new(before));
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        *document = *before;
        Ok(())
    }
}
