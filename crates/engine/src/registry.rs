// SPDX-License-Identifier: AGPL-3.0-or-later
//! Builds commands by name from JSON arguments, and describes each command's
//! arguments so front ends can generate forms and validate input.

use std::collections::BTreeMap;

use cndt_model::{Parameter, Unit};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{Command, CommandError, RemoveParameter, RenameDocument, SetParameter};

/// The type of a command argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgKind {
    /// A string.
    Text,
    /// A finite number.
    Number,
    /// A unit symbol: `mm`, `deg` or `none`.
    Unit,
}

/// One argument a command accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ArgSpec {
    /// The argument's key in the JSON object.
    pub name: &'static str,
    /// Its type.
    pub kind: ArgKind,
    /// Whether it must be given.
    pub required: bool,
    /// What it means, for help text and forms.
    pub description: &'static str,
}

type Builder = fn(&Map<String, Value>) -> Result<Box<dyn Command>, String>;

/// A registered command: its name, description, arguments and builder.
#[derive(Debug, Clone, Serialize)]
pub struct CommandSpec {
    /// The name commands are built by, such as `parameter.set`.
    pub name: &'static str,
    /// What the command does.
    pub description: &'static str,
    /// The arguments it accepts.
    pub args: &'static [ArgSpec],
    #[serde(skip)]
    build: Builder,
}

/// Every command that can be built by name.
#[derive(Debug, Clone)]
pub struct Registry {
    commands: BTreeMap<&'static str, CommandSpec>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::standard()
    }
}

impl Registry {
    /// The registry of built-in commands.
    #[must_use]
    pub fn standard() -> Self {
        let specs = [
            CommandSpec {
                name: "document.rename",
                description: "Rename the document.",
                args: &[ArgSpec {
                    name: "name",
                    kind: ArgKind::Text,
                    required: true,
                    description: "The new name.",
                }],
                build: |args| Ok(Box::new(RenameDocument::new(&text(args, "name")?))),
            },
            CommandSpec {
                name: "parameter.set",
                description: "Create a parameter, or change an existing one.",
                args: &[
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: true,
                        description: "Letters, digits and underscores, starting with a letter.",
                    },
                    ArgSpec {
                        name: "value",
                        kind: ArgKind::Number,
                        required: true,
                        description: "The value. Lengths are millimetres.",
                    },
                    ArgSpec {
                        name: "unit",
                        kind: ArgKind::Unit,
                        required: true,
                        description: "mm, deg or none.",
                    },
                    ArgSpec {
                        name: "comment",
                        kind: ArgKind::Text,
                        required: false,
                        description: "An optional note.",
                    },
                ],
                build: |args| {
                    let unit_symbol = text(args, "unit")?;
                    let unit = Unit::from_symbol(&unit_symbol).ok_or_else(|| {
                        format!("unknown unit {unit_symbol:?}; use mm, deg or none")
                    })?;
                    let mut parameter = Parameter::new(number(args, "value")?, unit);
                    if args.contains_key("comment") {
                        parameter.comment = text(args, "comment")?;
                    }
                    Ok(Box::new(SetParameter::new(&text(args, "name")?, parameter)))
                },
            },
            CommandSpec {
                name: "parameter.remove",
                description: "Delete a parameter.",
                args: &[ArgSpec {
                    name: "name",
                    kind: ArgKind::Text,
                    required: true,
                    description: "The parameter to delete.",
                }],
                build: |args| Ok(Box::new(RemoveParameter::new(&text(args, "name")?))),
            },
        ];
        Self {
            commands: specs.into_iter().map(|spec| (spec.name, spec)).collect(),
        }
    }

    /// Every registered command, sorted by name.
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.values()
    }

    /// One command's description.
    #[must_use]
    pub fn spec(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name)
    }

    /// Builds a command from a JSON object of arguments.
    ///
    /// # Errors
    ///
    /// Returns an error if no command has this name, the arguments are not a
    /// JSON object, a required argument is missing, an argument has the wrong
    /// type, or an argument is not one the command accepts.
    pub fn build(&self, name: &str, args: &Value) -> Result<Box<dyn Command>, CommandError> {
        let spec = self
            .commands
            .get(name)
            .ok_or_else(|| CommandError::UnknownCommand(name.to_owned()))?;
        let invalid = |reason: String| CommandError::InvalidArguments {
            command: name.to_owned(),
            reason,
        };
        let empty = Map::new();
        let args = match args {
            Value::Object(map) => map,
            Value::Null => &empty,
            _ => return Err(invalid(String::from("arguments must be a JSON object"))),
        };
        for key in args.keys() {
            if !spec.args.iter().any(|arg| arg.name == key) {
                return Err(invalid(format!("unexpected argument {key:?}")));
            }
        }
        for arg in spec.args.iter().filter(|arg| arg.required) {
            if !args.contains_key(arg.name) {
                return Err(invalid(format!("missing argument {:?}", arg.name)));
            }
        }
        (spec.build)(args).map_err(invalid)
    }
}

fn text(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("{key:?} must be a string")),
        None => Err(format!("missing argument {key:?}")),
    }
}

fn number(args: &Map<String, Value>, key: &str) -> Result<f64, String> {
    match args.get(key).and_then(Value::as_f64) {
        Some(value) if value.is_finite() => Ok(value),
        Some(_) => Err(format!("{key:?} must be finite")),
        None => Err(format!("{key:?} must be a number")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use cndt_model::Document;
    use serde_json::json;

    #[test]
    fn builds_and_runs_a_command_from_json() {
        let registry = Registry::standard();
        let mut session = Session::new(Document::default());
        let command = registry
            .build(
                "parameter.set",
                &json!({"name": "thickness", "value": 12, "unit": "mm"}),
            )
            .unwrap();
        session.execute(command).unwrap();
        let parameter = session.document().parameter("thickness").unwrap();
        assert!((parameter.value - 12.0).abs() < f64::EPSILON);
        assert_eq!(parameter.unit, Unit::Millimetre);
    }

    #[test]
    fn rejects_bad_input_with_a_reason() {
        let registry = Registry::standard();
        let cases = [
            ("nope", json!({}), "no command named nope"),
            ("parameter.set", json!([]), "must be a JSON object"),
            (
                "parameter.set",
                json!({"name": "a", "unit": "mm"}),
                "missing argument \"value\"",
            ),
            (
                "parameter.set",
                json!({"name": "a", "value": "1", "unit": "mm"}),
                "must be a number",
            ),
            (
                "parameter.set",
                json!({"name": "a", "value": 1, "unit": "in"}),
                "unknown unit",
            ),
            (
                "document.rename",
                json!({"name": "a", "colour": "red"}),
                "unexpected argument",
            ),
        ];
        for (name, args, expected) in cases {
            let error = registry.build(name, &args).unwrap_err().to_string();
            assert!(error.contains(expected), "{name} {args}: {error}");
        }
    }

    #[test]
    fn every_spec_names_match_the_command_it_builds() {
        let registry = Registry::standard();
        let samples = [
            ("document.rename", json!({"name": "x"})),
            (
                "parameter.set",
                json!({"name": "x", "value": 1, "unit": "none"}),
            ),
            ("parameter.remove", json!({"name": "x"})),
        ];
        assert_eq!(
            registry.specs().count(),
            samples.len(),
            "add a sample for each command"
        );
        for (name, args) in samples {
            assert_eq!(registry.build(name, &args).unwrap().name(), name);
        }
    }

    #[test]
    fn specs_serialise_for_front_ends() {
        let registry = Registry::standard();
        let json = serde_json::to_value(registry.spec("parameter.set").unwrap()).unwrap();
        assert_eq!(json["args"][2]["kind"], "unit");
        assert!(json.get("build").is_none());
    }
}
