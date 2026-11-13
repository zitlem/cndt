// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 2 project model: the document a user designs and machines.
//!
//! The document only changes through commands in the engine (architecture
//! rule 2). The engine's session owns the document and hands out shared
//! references, so code outside a command cannot reach the mutating methods.
//!
//! The document holds a name, user parameters, sketches and the timeline of
//! solid features. The part itself is not stored: [`rebuild`] makes it from
//! the timeline. CAM data arrives later.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub mod cam;
pub mod expr;
mod features;
pub mod threads;

pub use cndt_sketch::{self as sketch, Sketch, SketchError};
pub use features::{
    Counterbore, EdgeTarget, FaceName, FaceOrigin, Feature, FeatureEntry, FeatureId, FeatureStatus,
    Frame, FrameStatus, HoleDepth, HoleSpec, HoleSurface, Operation, Rebuilt, Thread,
    default_profiles, rebuild,
};
use serde::{Deserialize, Serialize};

/// Longest allowed parameter name, in characters.
pub const MAX_PARAMETER_NAME: usize = 64;

/// Longest allowed document name, in characters.
pub const MAX_DOCUMENT_NAME: usize = 200;

/// What a parameter's value measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// A length in millimetres.
    Millimetre,
    /// An angle in degrees.
    Degree,
    /// A plain number, such as a count.
    Unitless,
}

impl Unit {
    /// The short symbol shown after a value: `mm`, `deg`, or nothing.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Millimetre => "mm",
            Self::Degree => "deg",
            Self::Unitless => "",
        }
    }

    /// Parses a symbol as written by [`Unit::symbol`]. `none` also means
    /// unitless.
    #[must_use]
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "mm" => Some(Self::Millimetre),
            "deg" => Some(Self::Degree),
            "" | "none" => Some(Self::Unitless),
            _ => None,
        }
    }
}

/// A named value the design can refer to, such as `thickness = 12 mm`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// The value, in the parameter's unit. Lengths are millimetres.
    pub value: f64,
    /// What the value measures.
    pub unit: Unit,
    /// An optional note for the user.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

impl Parameter {
    /// Creates a parameter without a comment.
    #[must_use]
    pub const fn new(value: f64, unit: Unit) -> Self {
        Self {
            value,
            unit,
            comment: String::new(),
        }
    }
}

/// Identifies a sketch within a document. Never reused after deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SketchId(pub u32);

impl fmt::Display for SketchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sketch {}", self.0)
    }
}

/// The plane a sketch is drawn on: an origin plane, or a flat face of the
/// part named by its history (decision 0012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plane {
    /// The XY origin plane, seen from +Z. Sketch X is model X.
    Xy,
    /// The XZ origin plane, seen from −Y. Sketch X is model X, sketch Y is
    /// model Z.
    Xz,
    /// The YZ origin plane, seen from +X. Sketch X is model Y, sketch Y is
    /// model Z.
    Yz,
    /// A flat face of the part, by its persistent name, such as
    /// `f0/end/4-5-6-7`. Sketch axes follow the model's: the origin and X
    /// axis are the model's projected onto the face, and the normal points
    /// out of the part.
    Face(String),
}

impl Plane {
    /// Parses `xy`, `xz` or `yz`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "xy" => Some(Self::Xy),
            "xz" => Some(Self::Xz),
            "yz" => Some(Self::Yz),
            _ => None,
        }
    }
}

/// A named sketch on a plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchRecord {
    /// Shown in the browser tree, such as `Sketch1`.
    pub name: String,
    /// Where it is drawn.
    pub plane: Plane,
    /// Its geometry and constraints.
    pub sketch: Sketch,
    /// Dimensions driven by an expression, such as `thickness / 2`. The
    /// constraint holds the evaluated value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub expressions: BTreeMap<sketch::ConstraintId, String>,
}

/// Why a change to the model was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    /// A document or parameter name breaks the naming rules.
    InvalidName {
        /// The rejected name.
        name: String,
        /// Which rule it breaks.
        reason: &'static str,
    },
    /// A parameter value is infinite or not a number.
    NotFinite {
        /// The parameter's name.
        name: String,
    },
    /// No parameter has this name.
    UnknownParameter(String),
    /// No sketch has this id.
    UnknownSketch(SketchId),
    /// The sketch refused the change.
    Sketch(SketchError),
    /// No feature has this id.
    UnknownFeature(FeatureId),
    /// A CAM definition is not allowed; says why.
    InvalidCam(String),
    /// A feature's values are not allowed; says why.
    InvalidFeature(String),
    /// An expression could not be evaluated.
    Expression {
        /// The expression.
        text: String,
        /// Why.
        reason: String,
    },
    /// Expressions still use this parameter.
    ParameterInUse {
        /// The parameter.
        name: String,
        /// What uses it.
        by: Vec<String>,
    },
    /// Features still use this sketch.
    SketchInUse {
        /// The sketch's name.
        sketch: String,
        /// The features that use it.
        by: Vec<String>,
    },
}

impl From<SketchError> for ModelError {
    fn from(error: SketchError) -> Self {
        Self::Sketch(error)
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { name, reason } => write!(f, "invalid name {name:?}: {reason}"),
            Self::NotFinite { name } => write!(f, "parameter {name} must be a finite number"),
            Self::UnknownParameter(name) => write!(f, "there is no parameter named {name}"),
            Self::UnknownSketch(id) => write!(f, "{id} does not exist"),
            Self::Sketch(error) => error.fmt(f),
            Self::UnknownFeature(id) => write!(f, "{id} does not exist"),
            Self::InvalidCam(reason) => f.write_str(reason),
            Self::Expression { text, reason } => write!(f, "cannot work out {text:?}: {reason}"),
            Self::ParameterInUse { name, by } => {
                write!(f, "{name} is used by {}; change those first", by.join(", "))
            }
            Self::InvalidFeature(reason) => f.write_str(reason),
            Self::SketchInUse { sketch, by } => write!(
                f,
                "{sketch} is used by {}; delete or change those first",
                by.join(", ")
            ),
        }
    }
}

impl std::error::Error for ModelError {}

/// A CNDT project document.
///
/// Unknown fields are refused when reading, so a file from a newer build is
/// never opened and then saved without the parts this build does not know
/// (decision 0009, "never drop data").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    name: String,
    #[serde(default)]
    parameters: BTreeMap<String, Parameter>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sketches: BTreeMap<SketchId, SketchRecord>,
    #[serde(default, skip_serializing_if = "is_zero")]
    next_sketch: u32,
    /// The timeline, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    features: Vec<FeatureEntry>,
    #[serde(default, skip_serializing_if = "is_zero")]
    next_feature: u32,
    /// Stock, tools, setups and operations (decision 0014).
    #[serde(default, skip_serializing_if = "cam::Cam::is_empty")]
    cam: cam::Cam,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes fields by reference
const fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

impl Document {
    /// Creates an empty document. An invalid name falls back to `Untitled`.
    #[must_use]
    pub fn new(name: &str) -> Self {
        let name = if check_document_name(name).is_ok() {
            name.to_owned()
        } else {
            String::from("Untitled")
        };
        Self {
            name,
            parameters: BTreeMap::new(),
            sketches: BTreeMap::new(),
            next_sketch: 0,
            features: Vec::new(),
            next_feature: 0,
            cam: cam::Cam::default(),
        }
    }

    /// The document's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// All parameters, sorted by name.
    #[must_use]
    pub const fn parameters(&self) -> &BTreeMap<String, Parameter> {
        &self.parameters
    }

    /// One parameter by name.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.get(name)
    }

    /// Renames the document and returns the old name.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::InvalidName`] if the name is empty, too long or
    /// contains control characters. The document is unchanged on error.
    pub fn set_name(&mut self, name: &str) -> Result<String, ModelError> {
        check_document_name(name)?;
        Ok(std::mem::replace(&mut self.name, name.to_owned()))
    }

    /// Creates or replaces a parameter and returns the one it replaced.
    ///
    /// # Errors
    ///
    /// Returns an error if the name breaks the naming rules or the value is
    /// not finite. The document is unchanged on error.
    pub fn set_parameter(
        &mut self,
        name: &str,
        parameter: Parameter,
    ) -> Result<Option<Parameter>, ModelError> {
        check_parameter_name(name)?;
        if !parameter.value.is_finite() {
            return Err(ModelError::NotFinite {
                name: name.to_owned(),
            });
        }
        // Everything that uses the parameter follows it, or nothing changes.
        let mut trial = self.clone();
        let previous = trial.parameters.insert(name.to_owned(), parameter);
        trial.reevaluate()?;
        *self = trial;
        Ok(previous)
    }

    /// Evaluates an expression against the parameters.
    ///
    /// # Errors
    ///
    /// [`ModelError::Expression`] with the reason.
    pub fn evaluate(&self, text: &str) -> Result<f64, ModelError> {
        expr::evaluate(text, &self.parameters).map_err(|reason| ModelError::Expression {
            text: text.to_owned(),
            reason,
        })
    }

    /// What uses a parameter: dimensions and features, by name.
    fn users_of(&self, name: &str) -> Vec<String> {
        let uses = |text: &str| expr::names(text).iter().any(|n| n == name);
        let mut by = Vec::new();
        for record in self.sketches.values() {
            for (id, text) in &record.expressions {
                if uses(text) {
                    by.push(format!("{} dimension {}", record.name, id.0));
                }
            }
        }
        for entry in &self.features {
            if let Feature::Extrude {
                name: feature,
                distance_expression: Some(text),
                ..
            } = &entry.feature
                && uses(text)
            {
                by.push(feature.clone());
            }
        }
        by
    }

    /// Re-evaluates every expression and applies the values: dimensions are
    /// re-solved, feature values updated. Expressions on constraints that
    /// no longer exist are dropped.
    fn reevaluate(&mut self) -> Result<(), ModelError> {
        let parameters = self.parameters.clone();
        let eval = |text: &str| {
            expr::evaluate(text, &parameters).map_err(|reason| ModelError::Expression {
                text: text.to_owned(),
                reason,
            })
        };
        for record in self.sketches.values_mut() {
            let sketch = &record.sketch;
            record
                .expressions
                .retain(|id, _| sketch.constraint(*id).is_some());
            for (id, text) in &record.expressions {
                let value = eval(text)?;
                let current = record
                    .sketch
                    .constraint(*id)
                    .and_then(sketch::Constraint::value);
                if current.is_none_or(|c| (c - value).abs() > 1e-12) {
                    record
                        .sketch
                        .set_dimension(*id, value, &sketch::SolveOptions::default())?;
                }
            }
        }
        for entry in &mut self.features {
            if let Feature::Extrude {
                distance,
                distance_expression: Some(text),
                ..
            } = &mut entry.feature
            {
                *distance = eval(text)?;
            }
        }
        Ok(())
    }

    /// Drives a sketch dimension by an expression, or by a plain value again
    /// with `None`. Returns the sketch as it was, for undo.
    ///
    /// # Errors
    ///
    /// The expression's or the sketch's reason; nothing changes then.
    pub fn set_dimension_expression(
        &mut self,
        sketch: SketchId,
        constraint: sketch::ConstraintId,
        text: Option<&str>,
    ) -> Result<SketchRecord, ModelError> {
        let before = self
            .sketches
            .get(&sketch)
            .cloned()
            .ok_or(ModelError::UnknownSketch(sketch))?;
        if before
            .sketch
            .constraint(constraint)
            .and_then(sketch::Constraint::value)
            .is_none()
        {
            return Err(ModelError::Sketch(SketchError::NotADimension(constraint)));
        }
        let mut trial = self.clone();
        if let Some(record) = trial.sketches.get_mut(&sketch) {
            match text {
                Some(text) => {
                    record
                        .expressions
                        .insert(constraint, text.trim().to_owned());
                }
                None => {
                    record.expressions.remove(&constraint);
                }
            }
        }
        trial.reevaluate()?;
        *self = trial;
        Ok(before)
    }

    /// Puts a sketch back exactly as it was, expressions included, as undo
    /// does.
    pub fn set_sketch_record(&mut self, id: SketchId, record: SketchRecord) {
        self.sketches.insert(id, record);
    }

    /// Removes a parameter and returns it.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownParameter`] if no parameter has this name.
    pub fn remove_parameter(&mut self, name: &str) -> Result<Parameter, ModelError> {
        let by = self.users_of(name);
        if !by.is_empty() {
            return Err(ModelError::ParameterInUse {
                name: name.to_owned(),
                by,
            });
        }
        self.parameters
            .remove(name)
            .ok_or_else(|| ModelError::UnknownParameter(name.to_owned()))
    }

    /// All sketches, in id order.
    #[must_use]
    pub const fn sketches(&self) -> &BTreeMap<SketchId, SketchRecord> {
        &self.sketches
    }

    /// One sketch.
    #[must_use]
    pub fn sketch(&self, id: SketchId) -> Option<&SketchRecord> {
        self.sketches.get(&id)
    }

    /// Adds an empty sketch. An empty name gets the next free `SketchN`.
    /// A face plane is checked for form only; whether the face exists is
    /// known at rebuild.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::InvalidName`] for a name with control
    /// characters or over 200 characters.
    pub fn add_sketch(&mut self, name: &str, plane: Plane) -> Result<SketchId, ModelError> {
        let id = SketchId(self.next_sketch);
        let name = if name.trim().is_empty() {
            (1..)
                .map(|n| format!("Sketch{n}"))
                .find(|candidate| self.sketches.values().all(|s| s.name != *candidate))
                .unwrap_or_default()
        } else {
            check_document_name(name)?;
            name.to_owned()
        };
        check_plane(&plane)?;
        self.restore_sketch(
            id,
            SketchRecord {
                expressions: BTreeMap::new(),
                name,
                plane,
                sketch: Sketch::new(),
            },
        );
        Ok(id)
    }

    /// Puts a sketch back under its old id, as undoing a deletion does.
    pub fn restore_sketch(&mut self, id: SketchId, record: SketchRecord) {
        self.next_sketch = self.next_sketch.max(id.0.saturating_add(1));
        self.sketches.insert(id, record);
    }

    /// Deletes a sketch and returns it.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownSketch`] if there is no such sketch.
    pub fn remove_sketch(&mut self, id: SketchId) -> Result<SketchRecord, ModelError> {
        let record = self
            .sketches
            .get(&id)
            .ok_or(ModelError::UnknownSketch(id))?;
        let by: Vec<String> = self
            .features
            .iter()
            .filter(|e| e.feature.sketch() == Some(id))
            .map(|e| e.feature.name().to_owned())
            .collect();
        if !by.is_empty() {
            return Err(ModelError::SketchInUse {
                sketch: record.name.clone(),
                by,
            });
        }
        self.sketches
            .remove(&id)
            .ok_or(ModelError::UnknownSketch(id))
    }

    /// The CAM definitions.
    #[must_use]
    pub const fn cam(&self) -> &cam::Cam {
        &self.cam
    }

    /// Changes the CAM definitions. Returns the id of anything created.
    ///
    /// # Errors
    ///
    /// [`ModelError::InvalidCam`] with the reason; nothing changes then.
    pub fn edit_cam(&mut self, edit: cam::CamEdit) -> Result<Option<u32>, ModelError> {
        self.cam.apply(edit).map_err(ModelError::InvalidCam)
    }

    /// The timeline, in order.
    #[must_use]
    pub fn features(&self) -> &[FeatureEntry] {
        &self.features
    }

    /// One feature.
    #[must_use]
    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features
            .iter()
            .find(|e| e.id == id)
            .map(|e| &e.feature)
    }

    /// The next free `<prefix>N` feature name.
    #[must_use]
    pub fn next_feature_name(&self, prefix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}"))
            .find(|candidate| self.features.iter().all(|e| e.feature.name() != candidate))
            .unwrap_or_default()
    }

    /// Adds a feature at the end of the timeline.
    ///
    /// # Errors
    ///
    /// [`ModelError::InvalidFeature`] for bad values, or
    /// [`ModelError::UnknownSketch`].
    pub fn add_feature(&mut self, feature: Feature) -> Result<FeatureId, ModelError> {
        self.check_feature(&feature)?;
        let id = FeatureId(self.next_feature);
        self.next_feature = self.next_feature.saturating_add(1);
        self.features.push(FeatureEntry { id, feature });
        Ok(id)
    }

    /// Replaces a feature's definition, keeping its place, and returns the
    /// old one.
    ///
    /// # Errors
    ///
    /// As for [`Document::add_feature`], or [`ModelError::UnknownFeature`].
    pub fn replace_feature(
        &mut self,
        id: FeatureId,
        feature: Feature,
    ) -> Result<Feature, ModelError> {
        self.check_feature(&feature)?;
        let entry = self
            .features
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(ModelError::UnknownFeature(id))?;
        Ok(std::mem::replace(&mut entry.feature, feature))
    }

    /// Removes a feature, returning its timeline position and entry.
    ///
    /// # Errors
    ///
    /// [`ModelError::UnknownFeature`].
    pub fn remove_feature(&mut self, id: FeatureId) -> Result<(usize, FeatureEntry), ModelError> {
        let index = self
            .features
            .iter()
            .position(|e| e.id == id)
            .ok_or(ModelError::UnknownFeature(id))?;
        Ok((index, self.features.remove(index)))
    }

    /// Puts a removed feature back where it was, as undo does.
    pub fn restore_feature(&mut self, index: usize, entry: FeatureEntry) {
        self.next_feature = self.next_feature.max(entry.id.0.saturating_add(1));
        let index = index.min(self.features.len());
        self.features.insert(index, entry);
    }

    fn check_feature(&self, feature: &Feature) -> Result<(), ModelError> {
        feature.check().map_err(ModelError::InvalidFeature)?;
        check_document_name(feature.name())?;
        match feature.sketch() {
            Some(sketch) if !self.sketches.contains_key(&sketch) => {
                Err(ModelError::UnknownSketch(sketch))
            }
            _ => Ok(()),
        }
    }

    /// Changes a sketch's contents. The edit runs on a copy, and the
    /// sketch only changes if it succeeds.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownSketch`], or the edit's own error.
    pub fn edit_sketch<T>(
        &mut self,
        id: SketchId,
        edit: impl FnOnce(&mut Sketch) -> Result<T, SketchError>,
    ) -> Result<T, ModelError> {
        let record = self
            .sketches
            .get_mut(&id)
            .ok_or(ModelError::UnknownSketch(id))?;
        let mut copy = record.sketch.clone();
        let out = edit(&mut copy)?;
        record.sketch = copy;
        Ok(out)
    }

    /// Replaces a sketch's contents wholesale, returning the old ones. Used
    /// by undo.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownSketch`] if there is no such sketch.
    pub fn replace_sketch(&mut self, id: SketchId, sketch: Sketch) -> Result<Sketch, ModelError> {
        let record = self
            .sketches
            .get_mut(&id)
            .ok_or(ModelError::UnknownSketch(id))?;
        Ok(std::mem::replace(&mut record.sketch, sketch))
    }

    /// Checks every rule the setters enforce. Used after loading a file,
    /// which bypasses the setters.
    ///
    /// # Errors
    ///
    /// Returns the first broken rule found.
    pub fn validate(&self) -> Result<(), ModelError> {
        check_document_name(&self.name)?;
        for (name, parameter) in &self.parameters {
            check_parameter_name(name)?;
            if !parameter.value.is_finite() {
                return Err(ModelError::NotFinite { name: name.clone() });
            }
        }
        for (id, record) in &self.sketches {
            if id.0 >= self.next_sketch {
                return Err(ModelError::Sketch(SketchError::Corrupt(format!(
                    "{id} is beyond the next free sketch id"
                ))));
            }
            check_document_name(&record.name)?;
            check_plane(&record.plane)?;
            record.sketch.validate()?;
            for text in record.expressions.values() {
                self.evaluate(text)?;
            }
        }
        self.cam.check().map_err(ModelError::InvalidCam)?;
        let mut ids = BTreeSet::new();
        for entry in &self.features {
            if entry.id.0 >= self.next_feature || !ids.insert(entry.id) {
                return Err(ModelError::InvalidFeature(format!(
                    "{} has a damaged id",
                    entry.feature.name()
                )));
            }
            self.check_feature(&entry.feature)?;
        }
        Ok(())
    }
}

fn check_document_name(name: &str) -> Result<(), ModelError> {
    let reason = if name.trim().is_empty() {
        Some("it is empty")
    } else if name.chars().count() > MAX_DOCUMENT_NAME {
        Some("it is longer than 200 characters")
    } else if name.chars().any(char::is_control) {
        Some("it contains control characters")
    } else {
        None
    };
    reason.map_or(Ok(()), |reason| {
        Err(ModelError::InvalidName {
            name: name.to_owned(),
            reason,
        })
    })
}

fn check_plane(plane: &Plane) -> Result<(), ModelError> {
    match plane {
        Plane::Face(name)
            if name.is_empty() || name.len() > 200 || name.chars().any(char::is_control) =>
        {
            Err(ModelError::InvalidName {
                name: name.clone(),
                reason: "it is not a face name",
            })
        }
        _ => Ok(()),
    }
}

fn check_parameter_name(name: &str) -> Result<(), ModelError> {
    let mut chars = name.chars();
    let reason = match chars.next() {
        None => Some("it is empty"),
        Some(first) if !first.is_ascii_alphabetic() => Some("it must start with a letter"),
        Some(_) if name.len() > MAX_PARAMETER_NAME => Some("it is longer than 64 characters"),
        Some(_) if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            Some("only letters, digits and underscores are allowed")
        }
        Some(_) => None,
    };
    reason.map_or(Ok(()), |reason| {
        Err(ModelError::InvalidName {
            name: name.to_owned(),
            reason,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_replace_a_parameter() {
        let mut document = Document::new("Bracket");
        let first = Parameter::new(12.0, Unit::Millimetre);
        assert_eq!(document.set_parameter("thickness", first.clone()), Ok(None));
        let second = Parameter::new(15.0, Unit::Millimetre);
        assert_eq!(
            document.set_parameter("thickness", second.clone()),
            Ok(Some(first))
        );
        assert_eq!(document.parameter("thickness"), Some(&second));
    }

    #[test]
    fn bad_parameter_names_are_refused_and_change_nothing() {
        let mut document = Document::new("Bracket");
        for bad in ["", "1st", "has space", "dash-ed", &"a".repeat(65)] {
            let result = document.set_parameter(bad, Parameter::new(1.0, Unit::Unitless));
            assert!(
                matches!(result, Err(ModelError::InvalidName { .. })),
                "{bad:?}"
            );
        }
        assert!(document.parameters().is_empty());
    }

    #[test]
    fn non_finite_values_are_refused() {
        let mut document = Document::default();
        for bad in [f64::NAN, f64::INFINITY] {
            let result = document.set_parameter("x", Parameter::new(bad, Unit::Millimetre));
            assert!(matches!(result, Err(ModelError::NotFinite { .. })));
        }
        assert!(document.parameters().is_empty());
    }

    #[test]
    fn invalid_document_names_are_refused() {
        let mut document = Document::new("Bracket");
        assert!(document.set_name("   ").is_err());
        assert!(document.set_name("tab\there").is_err());
        assert_eq!(document.name(), "Bracket");
        assert_eq!(document.set_name("Plate").as_deref(), Ok("Bracket"));
    }

    #[test]
    fn sketches_are_added_named_and_edited_atomically() {
        let mut document = Document::default();
        let first = document.add_sketch("", Plane::Xy).unwrap();
        let second = document.add_sketch("", Plane::Xz).unwrap();
        assert_eq!(document.sketch(first).unwrap().name, "Sketch1");
        assert_eq!(document.sketch(second).unwrap().name, "Sketch2");
        let point = document
            .edit_sketch(first, |s| s.add_point(sketch::Point2::new(1.0, 2.0)))
            .unwrap();
        // A failing edit leaves the sketch exactly as it was.
        let before = document.clone();
        let error = document.edit_sketch(first, |s| {
            s.add_point(sketch::Point2::new(5.0, 5.0))?;
            s.add_line(point, point)
        });
        assert!(matches!(
            error,
            Err(ModelError::Sketch(SketchError::SameEntity(_)))
        ));
        assert_eq!(document, before);
        // Deleted ids are not reused.
        let removed = document.remove_sketch(second).unwrap();
        assert_eq!(document.add_sketch("Top", Plane::Xy), Ok(SketchId(2)));
        document.restore_sketch(second, removed);
        assert_eq!(document.sketches().len(), 3);
        assert_eq!(document.validate(), Ok(()));
    }

    #[test]
    fn features_protect_their_sketch_and_restore_in_place() {
        let mut document = Document::default();
        let s = document.add_sketch("", Plane::Xy).unwrap();
        let extrude = |name: &str| Feature::Extrude {
            name: name.into(),
            sketch: s,
            profiles: vec![sketch::ProfileKey(vec![sketch::EntityId(4)])],
            distance: 10.0,
            distance_expression: None,
            operation: Operation::Join,
        };
        let a = document.add_feature(extrude("Extrude1")).unwrap();
        let b = document.add_feature(extrude("Extrude2")).unwrap();
        assert_eq!(document.next_feature_name("Extrude"), "Extrude3");
        assert!(matches!(
            document.remove_sketch(s),
            Err(ModelError::SketchInUse { .. })
        ));
        let (index, entry) = document.remove_feature(a).unwrap();
        document.restore_feature(index, entry);
        assert_eq!(document.features()[0].id, a);
        assert_eq!(document.features()[1].id, b);
        let mut zero = extrude("Bad");
        let Feature::Extrude { distance, .. } = &mut zero else {
            panic!()
        };
        *distance = 0.0;
        assert!(matches!(
            document.add_feature(zero),
            Err(ModelError::InvalidFeature(_))
        ));
        assert_eq!(document.validate(), Ok(()));
    }

    #[test]
    fn parameters_drive_dimensions_and_features() {
        let mut d = Document::default();
        d.set_parameter("width", Parameter::new(96.0, Unit::Millimetre))
            .unwrap();
        d.set_parameter("thickness", Parameter::new(20.0, Unit::Millimetre))
            .unwrap();
        let s = d.add_sketch("", Plane::Xy).unwrap();
        let (line, length) = d
            .edit_sketch(s, |k| {
                let a = k.add_point(sketch::Point2::new(0.0, 0.0))?;
                let b = k.add_point(sketch::Point2::new(90.0, 0.0))?;
                let line = k.add_line(a, b)?;
                let opts = sketch::SolveOptions::default();
                k.add_constraint(sketch::Constraint::Fix { point: a }, &opts)?;
                k.add_constraint(sketch::Constraint::Horizontal { line }, &opts)?;
                let length =
                    k.add_constraint(sketch::Constraint::Length { line, value: 90.0 }, &opts)?;
                Ok((line, length))
            })
            .unwrap();
        d.set_dimension_expression(s, length, Some("width"))
            .unwrap();
        let end = |d: &Document| {
            d.sketch(s)
                .unwrap()
                .sketch
                .point(sketch::EntityId(1))
                .unwrap()
                .x
        };
        assert!((end(&d) - 96.0).abs() < 1e-7);
        d.add_feature(Feature::Extrude {
            name: "Body".into(),
            sketch: s,
            profiles: vec![sketch::ProfileKey(vec![line])],
            distance: 1.0,
            distance_expression: Some("thickness / 2".into()),
            operation: Operation::Join,
        })
        .unwrap();
        // Changing a parameter moves everything that uses it.
        d.set_parameter("width", Parameter::new(120.0, Unit::Millimetre))
            .unwrap();
        d.set_parameter("thickness", Parameter::new(30.0, Unit::Millimetre))
            .unwrap();
        assert!((end(&d) - 120.0).abs() < 1e-7);
        let Feature::Extrude { distance, .. } = &d.features()[0].feature else {
            panic!()
        };
        assert!((distance - 15.0).abs() < 1e-12);
        // A parameter in use cannot be removed; an impossible value changes nothing.
        let error = d.remove_parameter("width").unwrap_err();
        assert!(error.to_string().contains("dimension"), "{error}");
        let before = d.clone();
        assert!(
            d.set_parameter("width", Parameter::new(-5.0, Unit::Millimetre))
                .is_err()
        );
        assert_eq!(d, before);
        // Unknown names are refused.
        let error = d
            .set_dimension_expression(s, length, Some("depth"))
            .unwrap_err();
        assert!(
            error.to_string().contains("no parameter named depth"),
            "{error}"
        );
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let json = r#"{"name":"x","bodies":{}}"#;
        assert!(serde_json::from_str::<Document>(json).is_err());
    }

    #[test]
    fn validate_catches_what_deserialising_lets_through() {
        let json = r#"{"name":"x","parameters":{"9lives":{"value":1.0,"unit":"unitless"}}}"#;
        let document: Document = serde_json::from_str(json).unwrap();
        assert!(document.validate().is_err());
    }
}
