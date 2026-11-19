// SPDX-License-Identifier: AGPL-3.0-or-later
//! Commands for work planes, work axes and work points (decision 0029).

use cndt_model::{Document, Reference, ReferenceId};

use crate::{Command, CommandError};

/// Adds a work plane, work axis or work point.
#[derive(Debug, Clone)]
pub struct AddWorkGeometry {
    reference: Reference,
    before: Option<Box<Document>>,
}

impl AddWorkGeometry {
    /// A command that adds `reference`. An empty name gets `PlaneN`,
    /// `AxisN` or `PointN`.
    #[must_use]
    pub const fn new(reference: Reference) -> Self {
        Self {
            reference,
            before: None,
        }
    }
}

impl Command for AddWorkGeometry {
    fn name(&self) -> &'static str {
        match self.reference {
            Reference::WorkPlane { .. } => "reference.add_work_plane",
            Reference::WorkAxis { .. } => "reference.add_work_axis",
            Reference::WorkPoint { .. } => "reference.add_work_point",
        }
    }

    fn label(&self) -> String {
        format!("Add {}", self.reference.kind())
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document.clone();
        document.add_reference(self.reference.clone())?;
        self.before = Some(Box::new(before));
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        *document = *before;
        Ok(())
    }
}

/// Deletes reference geometry nothing uses.
#[derive(Debug, Clone)]
pub struct DeleteWorkGeometry {
    id: ReferenceId,
    before: Option<Box<Document>>,
}

impl DeleteWorkGeometry {
    /// A command that deletes the reference geometry `id`.
    #[must_use]
    pub const fn new(id: ReferenceId) -> Self {
        Self { id, before: None }
    }
}

impl Command for DeleteWorkGeometry {
    fn name(&self) -> &'static str {
        "reference.delete"
    }

    fn label(&self) -> String {
        String::from("Delete reference geometry")
    }

    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = document.clone();
        document.remove_reference(self.id)?;
        self.before = Some(Box::new(before));
        Ok(())
    }

    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError> {
        let before = self.before.take().ok_or(CommandError::NotApplied)?;
        *document = *before;
        Ok(())
    }
}
