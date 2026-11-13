// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 4 command engine.
//!
//! Every change to a document is a [`Command`] run through a [`Session`]
//! (architecture rule 2). That one path gives undo and redo, scripting, the
//! CLI and headless testing. Commands are built by name from JSON arguments
//! through the [`Registry`], which is how the UI, the CLI and scripts reach
//! them.

mod commands;
mod feature_commands;
mod registry;
mod session;
mod sketch_commands;

use std::fmt;

use cndt_model::{Document, ModelError};

pub use commands::{RemoveParameter, RenameDocument, SetParameter};
pub use feature_commands::{
    AddBlend, AddExtrude, AddHole, BlendKind, DeleteFeature, EditBlend, EditCam, EditExtrude,
    EditHole, Grid, SetPattern,
};
pub use registry::{ArgKind, ArgSpec, CommandSpec, Registry};
pub use session::Session;
pub use sketch_commands::{CreateSketch, DeleteSketch, EditSketch, PointRef, SketchEdit};

/// A change to a document that can be undone.
///
/// The contract every command must keep:
/// - `apply` either succeeds completely or leaves the document unchanged.
/// - `undo` is only called after a successful `apply`, and restores the
///   document to exactly how it was before.
/// - `apply` may be called again after `undo` (redo) and must give the same
///   result.
pub trait Command: fmt::Debug + Send {
    /// The registered name, such as `parameter.set`.
    fn name(&self) -> &'static str;

    /// A short description for the undo menu, such as `Set thickness`.
    fn label(&self) -> String;

    /// Applies the change.
    ///
    /// # Errors
    ///
    /// Returns why the change was refused. The document is unchanged.
    fn apply(&mut self, document: &mut Document) -> Result<(), CommandError>;

    /// Reverses the last successful `apply`.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::NotApplied`] if `apply` has not succeeded.
    fn undo(&mut self, document: &mut Document) -> Result<(), CommandError>;
}

/// Why a command could not be built, applied or undone.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandError {
    /// No command is registered under this name.
    UnknownCommand(String),
    /// The arguments are missing, of the wrong type or unexpected.
    InvalidArguments {
        /// The command being built.
        command: String,
        /// What is wrong with the arguments.
        reason: String,
    },
    /// The model refused the change.
    Model(ModelError),
    /// `undo` was called on a command that has not been applied.
    NotApplied,
    /// There is nothing to undo.
    NothingToUndo,
    /// There is nothing to redo.
    NothingToRedo,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(name) => write!(f, "there is no command named {name}"),
            Self::InvalidArguments { command, reason } => {
                write!(f, "invalid arguments for {command}: {reason}")
            }
            Self::Model(error) => error.fmt(f),
            Self::NotApplied => write!(f, "the command has not been applied"),
            Self::NothingToUndo => write!(f, "there is nothing to undo"),
            Self::NothingToRedo => write!(f, "there is nothing to redo"),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<ModelError> for CommandError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}
