// SPDX-License-Identifier: AGPL-3.0-or-later
//! The built-in commands.

use cndt_model::{Document, Parameter};

use crate::{Command, CommandError};

/// Renames the document.
#[derive(Debug, Clone)]
pub struct RenameDocument {
    name: String,
    previous: Option<String>,
}

impl RenameDocument {
    /// A command that renames the document to `name`.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            previous: None,
        }
    }
}

impl Command for RenameDocument {
    fn name(&self) -> &'static str {
        "document.rename"
    }

    fn label(&self) -> String {
        String::from("Rename document")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        self.previous = Some(document.set_name(&self.name)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let previous = self.previous.take().ok_or(CommandError::NotApplied)?;
        document.set_name(&previous)?;
        Ok(())
    }
}

/// Creates or changes a parameter. Everything driven by it follows; undo
/// puts the whole document back exactly.
#[derive(Debug, Clone)]
pub struct SetParameter {
    name: String,
    parameter: Parameter,
    before: Option<Box<Document>>,
}

impl SetParameter {
    /// A command that sets `name` to `parameter`.
    #[must_use]
    pub fn new(name: &str, parameter: Parameter) -> Self {
        Self {
            name: name.to_owned(),
            parameter,
            before: None,
        }
    }
}

impl Command for SetParameter {
    fn name(&self) -> &'static str {
        "parameter.set"
    }

    fn label(&self) -> String {
        format!("Set {}", self.name)
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document.clone();
        document.set_parameter(&self.name, self.parameter.clone())?;
        self.before = Some(Box::new(before));
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        *document = *before;
        Ok(())
    }
}

/// Deletes a parameter.
#[derive(Debug, Clone)]
pub struct RemoveParameter {
    name: String,
    removed: Option<Parameter>,
}

impl RemoveParameter {
    /// A command that deletes the parameter `name`.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            removed: None,
        }
    }
}

impl Command for RemoveParameter {
    fn name(&self) -> &'static str {
        "parameter.remove"
    }

    fn label(&self) -> String {
        format!("Remove {}", self.name)
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        self.removed = Some(document.remove_parameter(&self.name)?);
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let removed = self.removed.take().ok_or(CommandError::NotApplied)?;
        document.set_parameter(&self.name, removed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cndt_model::Unit;

    #[test]
    fn each_command_round_trips() {
        let mut document = Document::new("Bracket");
        document
            .set_parameter("width", Parameter::new(80.0, Unit::Millimetre))
            .unwrap();
        let original = document.clone();

        let mut commands: Vec<Box<dyn Command>> = vec![
            Box::new(RenameDocument::new("Plate")),
            Box::new(SetParameter::new(
                "width",
                Parameter::new(90.0, Unit::Millimetre),
            )),
            Box::new(SetParameter::new(
                "holes",
                Parameter::new(4.0, Unit::Unitless),
            )),
            Box::new(RemoveParameter::new("width")),
        ];
        for command in &mut commands {
            let before = document.clone();
            command.apply(&mut document).unwrap();
            assert_ne!(document, before, "{} changed nothing", command.name());
            command.undo(&mut document).unwrap();
            assert_eq!(document, before, "{} did not undo cleanly", command.name());
        }
        assert_eq!(document, original);
    }

    #[test]
    fn undo_before_apply_is_refused() {
        let mut document = Document::default();
        let mut command = RemoveParameter::new("x");
        assert_eq!(command.undo(&mut document), Err(CommandError::NotApplied));
    }
}
