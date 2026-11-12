// SPDX-License-Identifier: AGPL-3.0-or-later
//! The editing session: one document, its undo and redo history, and whether
//! it has unsaved changes.

use cndt_model::Document;

use crate::{Command, CommandError};

#[derive(Debug)]
struct Entry {
    /// Unique within the session, so a saved state can be recognised even
    /// after undo and redo.
    id: u64,
    command: Box<dyn Command>,
}

/// An open document and its history.
///
/// The session owns the document and only hands out shared references, so
/// the only way to change it is [`Session::execute`].
#[derive(Debug)]
pub struct Session {
    document: Document,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: u64,
    /// The top of the undo stack when the document was last saved or opened.
    clean_top: Option<u64>,
}

impl Session {
    /// Starts a session on a document, which counts as saved.
    #[must_use]
    pub const fn new(document: Document) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            clean_top: None,
        }
    }

    /// The current document.
    #[must_use]
    pub const fn document(&self) -> &Document {
        &self.document
    }

    /// Applies a command and records it for undo. Clears the redo history.
    ///
    /// # Errors
    ///
    /// Returns the command's error. The document and history are unchanged.
    pub fn execute(&mut self, mut command: Box<dyn Command>) -> Result<(), CommandError> {
        command.apply(&mut self.document)?;
        let id = self.next_id;
        self.next_id += 1;
        self.undo.push(Entry { id, command });
        self.redo.clear();
        Ok(())
    }

    /// Undoes the most recent command.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::NothingToUndo`] if the history is empty, or the
    /// command's error, in which case the history is unchanged.
    pub fn undo(&mut self) -> Result<(), CommandError> {
        let mut entry = self.undo.pop().ok_or(CommandError::NothingToUndo)?;
        if let Err(error) = entry.command.undo(&mut self.document) {
            self.undo.push(entry);
            return Err(error);
        }
        self.redo.push(entry);
        Ok(())
    }

    /// Re-applies the most recently undone command.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::NothingToRedo`] if nothing was undone, or the
    /// command's error, in which case the history is unchanged.
    pub fn redo(&mut self) -> Result<(), CommandError> {
        let mut entry = self.redo.pop().ok_or(CommandError::NothingToRedo)?;
        if let Err(error) = entry.command.apply(&mut self.document) {
            self.redo.push(entry);
            return Err(error);
        }
        self.undo.push(entry);
        Ok(())
    }

    /// The label of the command [`Session::undo`] would reverse.
    #[must_use]
    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|entry| entry.command.label())
    }

    /// The label of the command [`Session::redo`] would re-apply.
    #[must_use]
    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|entry| entry.command.label())
    }

    /// Whether the document differs from when it was last saved or opened.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.undo.last().map(|entry| entry.id) != self.clean_top
    }

    /// Records that the document was just saved.
    pub fn mark_saved(&mut self) {
        self.clean_top = self.undo.last().map(|entry| entry.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenameDocument, SetParameter};
    use cndt_model::{Parameter, Unit};

    fn set(name: &str, value: f64) -> Box<dyn Command> {
        Box::new(SetParameter::new(
            name,
            Parameter::new(value, Unit::Millimetre),
        ))
    }

    fn value(session: &Session, name: &str) -> Option<f64> {
        session.document().parameter(name).map(|p| p.value)
    }

    #[test]
    fn execute_undo_redo() {
        let mut session = Session::new(Document::new("Bracket"));
        session.execute(set("thickness", 12.0)).unwrap();
        session.execute(set("thickness", 15.0)).unwrap();
        assert_eq!(value(&session, "thickness"), Some(15.0));
        assert_eq!(session.undo_label().as_deref(), Some("Set thickness"));

        session.undo().unwrap();
        assert_eq!(value(&session, "thickness"), Some(12.0));
        session.undo().unwrap();
        assert_eq!(value(&session, "thickness"), None);
        assert_eq!(session.undo(), Err(CommandError::NothingToUndo));

        session.redo().unwrap();
        session.redo().unwrap();
        assert_eq!(value(&session, "thickness"), Some(15.0));
        assert_eq!(session.redo(), Err(CommandError::NothingToRedo));
    }

    #[test]
    fn a_new_command_clears_redo() {
        let mut session = Session::new(Document::default());
        session.execute(set("a", 1.0)).unwrap();
        session.undo().unwrap();
        session.execute(set("b", 2.0)).unwrap();
        assert_eq!(session.redo(), Err(CommandError::NothingToRedo));
    }

    #[test]
    fn a_refused_command_changes_nothing() {
        let mut session = Session::new(Document::new("Bracket"));
        session.execute(set("a", 1.0)).unwrap();
        let before = session.document().clone();
        let result = session.execute(Box::new(RenameDocument::new("")));
        assert!(matches!(result, Err(CommandError::Model(_))));
        assert_eq!(session.document(), &before);
        assert_eq!(session.undo_label().as_deref(), Some("Set a"));
    }

    #[test]
    fn dirty_tracking_follows_undo_and_redo() {
        let mut session = Session::new(Document::default());
        assert!(!session.is_dirty());
        session.execute(set("a", 1.0)).unwrap();
        assert!(session.is_dirty());
        session.mark_saved();
        assert!(!session.is_dirty());
        session.undo().unwrap();
        assert!(session.is_dirty());
        session.redo().unwrap();
        assert!(!session.is_dirty());
        session.undo().unwrap();
        session.execute(set("a", 1.0)).unwrap();
        assert!(session.is_dirty(), "same content, but a different history");
    }
}
