// SPDX-License-Identifier: AGPL-3.0-or-later
//! Builds commands by name from JSON arguments, and describes each command's
//! arguments so front ends can generate forms and validate input.

use std::collections::BTreeMap;

use cndt_model::sketch::{Constraint, ConstraintId, EntityId, Point2};
use cndt_model::{Parameter, Plane, SketchId, Unit};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    AddExtrude, Command, CommandError, CreateSketch, DeleteFeature, DeleteSketch, EditExtrude,
    EditSketch, PointRef, RemoveParameter, RenameDocument, SetParameter, SketchEdit,
};
use cndt_model::sketch::ProfileKey;
use cndt_model::{FeatureId, Operation};

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
    /// A whole number id, such as a sketch, entity or constraint id.
    Id,
    /// `true` or `false`.
    Boolean,
    /// A sketch position `{"x": 1, "y": 2}` in millimetres, or where noted,
    /// the id of an existing sketch point instead.
    Point,
    /// A sketch plane: `xy`, `xz` or `yz`.
    Plane,
    /// A sketch constraint object, such as
    /// `{"type": "length", "line": 4, "value": 96}`.
    Constraint,
    /// `join` or `cut`.
    Operation,
    /// A list of profiles, each the list of its outer boundary's curve ids,
    /// such as `[[4, 5, 6, 7]]`.
    Profiles,
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
            CommandSpec {
                name: "feature.extrude",
                description: "Extrude sketch profiles into a solid, or cut them from it.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "distance",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres along the sketch plane's normal; negative goes the other way.",
                    },
                    ArgSpec {
                        name: "operation",
                        kind: ArgKind::Operation,
                        required: false,
                        description: "join (default) or cut.",
                    },
                    ArgSpec {
                        name: "profiles",
                        kind: ArgKind::Profiles,
                        required: false,
                        description: "Which regions; by default every region that is material, so a circle inside an outline is a hole.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next ExtrudeN.",
                    },
                ],
                build: |args| {
                    let operation = if args.contains_key("operation") {
                        operation(args)?
                    } else {
                        Operation::Join
                    };
                    let profiles = if args.contains_key("profiles") {
                        Some(profiles(args)?)
                    } else {
                        None
                    };
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(AddExtrude::new(
                        sketch_id(args)?,
                        profiles,
                        number(args, "distance")?,
                        operation,
                        name,
                    )))
                },
            },
            CommandSpec {
                name: "feature.edit_extrude",
                description: "Change an extrusion's distance, operation or profiles.",
                args: &[
                    FEATURE,
                    ArgSpec {
                        name: "distance",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Millimetres.",
                    },
                    ArgSpec {
                        name: "operation",
                        kind: ArgKind::Operation,
                        required: false,
                        description: "join or cut.",
                    },
                    ArgSpec {
                        name: "profiles",
                        kind: ArgKind::Profiles,
                        required: false,
                        description: "Which regions.",
                    },
                ],
                build: |args| {
                    let distance = if args.contains_key("distance") {
                        Some(number(args, "distance")?)
                    } else {
                        None
                    };
                    let op = if args.contains_key("operation") {
                        Some(operation(args)?)
                    } else {
                        None
                    };
                    let profiles = if args.contains_key("profiles") {
                        Some(profiles(args)?)
                    } else {
                        None
                    };
                    Ok(Box::new(EditExtrude::new(
                        FeatureId(id(args, "feature")?),
                        distance,
                        op,
                        profiles,
                    )))
                },
            },
            CommandSpec {
                name: "feature.delete",
                description: "Delete a feature from the timeline.",
                args: &[FEATURE],
                build: |args| {
                    Ok(Box::new(DeleteFeature::new(FeatureId(id(
                        args, "feature",
                    )?))))
                },
            },
            CommandSpec {
                name: "sketch.create",
                description: "Create an empty sketch on an origin plane.",
                args: &[
                    ArgSpec {
                        name: "plane",
                        kind: ArgKind::Plane,
                        required: true,
                        description: "xy, xz or yz.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next SketchN.",
                    },
                ],
                build: |args| {
                    let name = if args.contains_key("name") {
                        text(args, "name")?
                    } else {
                        String::new()
                    };
                    Ok(Box::new(CreateSketch::new(&name, plane(args, "plane")?)))
                },
            },
            CommandSpec {
                name: "sketch.delete",
                description: "Delete a sketch.",
                args: &[SKETCH],
                build: |args| Ok(Box::new(DeleteSketch::new(sketch_id(args)?))),
            },
            CommandSpec {
                name: "sketch.add_point",
                description: "Add a point.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "at",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position in millimetres.",
                    },
                ],
                build: |args| edit(args, SketchEdit::AddPoint(position(args, "at")?)),
            },
            CommandSpec {
                name: "sketch.add_line",
                description: "Add a line. Each end is a new position or an existing point's id.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "start",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position, or existing point id.",
                    },
                    ArgSpec {
                        name: "end",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position, or existing point id.",
                    },
                ],
                build: |args| {
                    let (start, end) = (point_ref(args, "start")?, point_ref(args, "end")?);
                    edit(args, SketchEdit::AddLine { start, end })
                },
            },
            CommandSpec {
                name: "sketch.add_circle",
                description: "Add a circle.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "center",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position, or existing point id.",
                    },
                    ArgSpec {
                        name: "radius",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres.",
                    },
                ],
                build: |args| {
                    let center = point_ref(args, "center")?;
                    edit(
                        args,
                        SketchEdit::AddCircle {
                            center,
                            radius: number(args, "radius")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.add_arc",
                description: "Add an arc, counter-clockwise from start to end around center. The end is brought onto the circle through the start.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "center",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position, or existing point id.",
                    },
                    ArgSpec {
                        name: "start",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position, or existing point id.",
                    },
                    ArgSpec {
                        name: "end",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Position, or existing point id.",
                    },
                ],
                build: |args| {
                    let center = point_ref(args, "center")?;
                    let (start, end) = (point_ref(args, "start")?, point_ref(args, "end")?);
                    edit(args, SketchEdit::AddArc { center, start, end })
                },
            },
            CommandSpec {
                name: "sketch.add_rectangle",
                description: "Add a rectangle from two opposite corners, with horizontal and vertical constraints.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "from",
                        kind: ArgKind::Point,
                        required: true,
                        description: "One corner, in millimetres.",
                    },
                    ArgSpec {
                        name: "to",
                        kind: ArgKind::Point,
                        required: true,
                        description: "The opposite corner.",
                    },
                ],
                build: |args| {
                    let (from, to) = (position(args, "from")?, position(args, "to")?);
                    edit(args, SketchEdit::AddRectangle { from, to })
                },
            },
            CommandSpec {
                name: "sketch.constrain",
                description: "Add a constraint or dimension. Refused if it conflicts with or repeats the existing ones.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "constraint",
                        kind: ArgKind::Constraint,
                        required: true,
                        description: "The constraint, with its type and references.",
                    },
                ],
                build: |args| {
                    let value = args.get("constraint").cloned().unwrap_or_default();
                    let constraint: Constraint =
                        serde_json::from_value(value).map_err(|error| {
                            format!("\"constraint\" is not a valid constraint: {error}")
                        })?;
                    edit(args, SketchEdit::Constrain(constraint))
                },
            },
            CommandSpec {
                name: "sketch.set_dimension",
                description: "Change a dimension's value.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "constraint",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The dimension's id.",
                    },
                    ArgSpec {
                        name: "value",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres, or degrees for an angle.",
                    },
                ],
                build: |args| {
                    let constraint = ConstraintId(id(args, "constraint")?);
                    edit(
                        args,
                        SketchEdit::SetDimension {
                            constraint,
                            value: number(args, "value")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.move_point",
                description: "Drag a point; the constraints keep holding.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "point",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The point's id.",
                    },
                    ArgSpec {
                        name: "to",
                        kind: ArgKind::Point,
                        required: true,
                        description: "Where to, in millimetres.",
                    },
                ],
                build: |args| {
                    let point = EntityId(id(args, "point")?);
                    edit(
                        args,
                        SketchEdit::MovePoint {
                            point,
                            to: position(args, "to")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.set_construction",
                description: "Mark an entity as construction geometry, or back.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entity",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The entity's id.",
                    },
                    ArgSpec {
                        name: "construction",
                        kind: ArgKind::Boolean,
                        required: true,
                        description: "true for construction.",
                    },
                ],
                build: |args| {
                    let entity = EntityId(id(args, "entity")?);
                    edit(
                        args,
                        SketchEdit::SetConstruction {
                            entity,
                            construction: boolean(args, "construction")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.remove_entity",
                description: "Delete an entity and its constraints. Deleting a point deletes what is drawn from it.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entity",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The entity's id.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::RemoveEntity(EntityId(id(args, "entity")?)),
                    )
                },
            },
            CommandSpec {
                name: "sketch.remove_constraint",
                description: "Delete a constraint or dimension.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "constraint",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The constraint's id.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::RemoveConstraint(ConstraintId(id(args, "constraint")?)),
                    )
                },
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

/// The `feature` argument of the timeline commands.
const FEATURE: ArgSpec = ArgSpec {
    name: "feature",
    kind: ArgKind::Id,
    required: true,
    description: "The feature's id.",
};

fn operation(args: &Map<String, Value>) -> Result<Operation, String> {
    match text(args, "operation")?.as_str() {
        "join" => Ok(Operation::Join),
        "cut" => Ok(Operation::Cut),
        other => Err(format!("unknown operation {other:?}; use join or cut")),
    }
}

fn profiles(args: &Map<String, Value>) -> Result<Vec<ProfileKey>, String> {
    serde_json::from_value(args.get("profiles").cloned().unwrap_or_default()).map_err(|_| {
        String::from("\"profiles\" must be a list of curve-id lists, like [[4, 5, 6, 7]]")
    })
}

/// The `sketch` argument every sketch command takes.
const SKETCH: ArgSpec = ArgSpec {
    name: "sketch",
    kind: ArgKind::Id,
    required: true,
    description: "The sketch's id.",
};

fn id(args: &Map<String, Value>, key: &str) -> Result<u32, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{key:?} must be a whole number id"))
}

fn sketch_id(args: &Map<String, Value>) -> Result<SketchId, String> {
    id(args, "sketch").map(SketchId)
}

fn boolean(args: &Map<String, Value>, key: &str) -> Result<bool, String> {
    args.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{key:?} must be true or false"))
}

fn plane(args: &Map<String, Value>, key: &str) -> Result<Plane, String> {
    let name = text(args, key)?;
    Plane::from_name(&name).ok_or_else(|| format!("unknown plane {name:?}; use xy, xz or yz"))
}

fn position(args: &Map<String, Value>, key: &str) -> Result<Point2, String> {
    let bad = || format!("{key:?} must be a position like {{\"x\": 1, \"y\": 2}}");
    let object = args.get(key).and_then(Value::as_object).ok_or_else(bad)?;
    if object.keys().any(|k| k != "x" && k != "y") {
        return Err(bad());
    }
    let x = number(object, "x").map_err(|_| bad())?;
    let y = number(object, "y").map_err(|_| bad())?;
    Ok(Point2::new(x, y))
}

fn point_ref(args: &Map<String, Value>, key: &str) -> Result<PointRef, String> {
    if args.get(key).is_some_and(Value::is_number) {
        return id(args, key).map(|value| PointRef::Existing(EntityId(value)));
    }
    position(args, key).map(PointRef::New).map_err(|_| {
        format!("{key:?} must be a position like {{\"x\": 1, \"y\": 2}} or a point id")
    })
}

fn edit(args: &Map<String, Value>, edit: SketchEdit) -> Result<Box<dyn Command>, String> {
    Ok(Box::new(EditSketch::new(sketch_id(args)?, edit)))
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
            ("sketch.create", json!({"plane": "top"}), "unknown plane"),
            (
                "feature.extrude",
                json!({"sketch": 0, "distance": 1, "operation": "add"}),
                "unknown operation",
            ),
            (
                "feature.extrude",
                json!({"sketch": 0, "distance": 1, "profiles": [1, 2]}),
                "curve-id lists",
            ),
            ("sketch.delete", json!({"sketch": -1}), "whole number id"),
            (
                "sketch.add_point",
                json!({"sketch": 0, "at": [1, 2]}),
                "must be a position",
            ),
            (
                "sketch.add_point",
                json!({"sketch": 0, "at": {"x": 1, "y": 2, "z": 3}}),
                "must be a position",
            ),
            (
                "sketch.add_line",
                json!({"sketch": 0, "start": "a", "end": 1}),
                "or a point id",
            ),
            (
                "sketch.constrain",
                json!({"sketch": 0, "constraint": {"type": "glue"}}),
                "not a valid constraint",
            ),
            (
                "sketch.constrain",
                json!({"sketch": 0, "constraint": {"type": "fix", "point": 1, "extra": 2}}),
                "not a valid constraint",
            ),
            (
                "sketch.set_construction",
                json!({"sketch": 0, "entity": 1, "construction": 1}),
                "true or false",
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
            ("feature.extrude", json!({"sketch": 0, "distance": 20})),
            (
                "feature.edit_extrude",
                json!({"feature": 0, "distance": 25, "operation": "cut", "profiles": [[4, 5, 6, 7]]}),
            ),
            ("feature.delete", json!({"feature": 0})),
            ("sketch.create", json!({"plane": "xy"})),
            ("sketch.delete", json!({"sketch": 0})),
            (
                "sketch.add_point",
                json!({"sketch": 0, "at": {"x": 1, "y": 2}}),
            ),
            (
                "sketch.add_line",
                json!({"sketch": 0, "start": 3, "end": {"x": 1, "y": 2}}),
            ),
            (
                "sketch.add_circle",
                json!({"sketch": 0, "center": {"x": 0, "y": 0}, "radius": 3.3}),
            ),
            (
                "sketch.add_arc",
                json!({"sketch": 0, "center": 0, "start": 1, "end": 2}),
            ),
            (
                "sketch.add_rectangle",
                json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 96, "y": 56}}),
            ),
            (
                "sketch.constrain",
                json!({"sketch": 0, "constraint": {"type": "length", "line": 4, "value": 96}}),
            ),
            (
                "sketch.set_dimension",
                json!({"sketch": 0, "constraint": 4, "value": 100}),
            ),
            (
                "sketch.move_point",
                json!({"sketch": 0, "point": 1, "to": {"x": 3, "y": 4}}),
            ),
            (
                "sketch.set_construction",
                json!({"sketch": 0, "entity": 1, "construction": true}),
            ),
            ("sketch.remove_entity", json!({"sketch": 0, "entity": 1})),
            (
                "sketch.remove_constraint",
                json!({"sketch": 0, "constraint": 1}),
            ),
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
