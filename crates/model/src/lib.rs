// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 2 project model: the document a user designs and machines.
//!
//! The document only changes through commands in the engine (architecture
//! rule 2). The engine's session owns the document and hands out shared
//! references, so code outside a command cannot reach the mutating methods.
//!
//! For now the document holds a name and user parameters. Sketches, features
//! and CAM data arrive in later phases.

use std::collections::BTreeMap;
use std::fmt;

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

/// Why a change to the model was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { name, reason } => write!(f, "invalid name {name:?}: {reason}"),
            Self::NotFinite { name } => write!(f, "parameter {name} must be a finite number"),
            Self::UnknownParameter(name) => write!(f, "there is no parameter named {name}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// A CNDT project document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    name: String,
    #[serde(default)]
    parameters: BTreeMap<String, Parameter>,
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
        Ok(self.parameters.insert(name.to_owned(), parameter))
    }

    /// Removes a parameter and returns it.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownParameter`] if no parameter has this name.
    pub fn remove_parameter(&mut self, name: &str) -> Result<Parameter, ModelError> {
        self.parameters
            .remove(name)
            .ok_or_else(|| ModelError::UnknownParameter(name.to_owned()))
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
    fn validate_catches_what_deserialising_lets_through() {
        let json = r#"{"name":"x","parameters":{"9lives":{"value":1.0,"unit":"unitless"}}}"#;
        let document: Document = serde_json::from_str(json).unwrap();
        assert!(document.validate().is_err());
    }
}
