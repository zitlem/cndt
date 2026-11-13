// SPDX-License-Identifier: AGPL-3.0-or-later
//! Builds commands by name from JSON arguments, and describes each command's
//! arguments so front ends can generate forms and validate input.

use std::collections::BTreeMap;

use cndt_model::sketch::{Constraint, ConstraintId, EntityId, Point2};
use cndt_model::{Parameter, Plane, SketchId, Unit};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    AddBlend, AddExtrude, AddHole, BlendKind, Command, CommandError, CreateSketch, DeleteFeature,
    DeleteSketch, EditBlend, EditExtrude, EditHole, EditSketch, PointRef, RemoveParameter,
    RenameDocument, SetParameter, SketchEdit,
};
use cndt_model::sketch::ProfileKey;
use cndt_model::{Counterbore, EdgeTarget, HoleDepth, HoleSpec, Thread};
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
    /// A list of ids, such as `[3, 8]`.
    Ids,
    /// A depth in millimetres, or `"through"`.
    Depth,
    /// A list of face names, such as `["f0/end/4-5-6-7"]`.
    Faces,
    /// A list of edges, each the names of its two faces, such as
    /// `[["f0/side/e4", "f0/side/e5"]]`.
    Edges,
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
                name: "feature.hole",
                description: "Drill holes at sketch points: simple, counterbored or tapped.",
                args: ADD_HOLE_ARGS,
                build: |args| {
                    let points = if args.contains_key("points") {
                        Some(ids(args, "points")?)
                    } else {
                        None
                    };
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(AddHole::new(
                        sketch_id(args)?,
                        points,
                        hole_spec(args)?,
                        name,
                    )))
                },
            },
            CommandSpec {
                name: "feature.edit_hole",
                description: "Change a hole's size and type. Give its full specification.",
                args: EDIT_HOLE_ARGS,
                build: |args| {
                    Ok(Box::new(EditHole::new(
                        FeatureId(id(args, "feature")?),
                        hole_spec(args)?,
                    )))
                },
            },
            CommandSpec {
                name: "feature.chamfer",
                description: "Bevel edges at 45°: every edge of the given faces, and single edges.",
                args: &[
                    BLEND_TARGETS[0],
                    BLEND_TARGETS[1],
                    ArgSpec {
                        name: "distance",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres back from the edge on each face.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next ChamferN.",
                    },
                ],
                build: |args| blend(args, BlendKind::Chamfer, "distance"),
            },
            CommandSpec {
                name: "feature.fillet",
                description: "Round edges: every edge of the given faces, and single edges.",
                args: &[
                    BLEND_TARGETS[0],
                    BLEND_TARGETS[1],
                    ArgSpec {
                        name: "radius",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next FilletN.",
                    },
                ],
                build: |args| blend(args, BlendKind::Fillet, "radius"),
            },
            CommandSpec {
                name: "feature.edit_blend",
                description: "Change a chamfer's distance or a fillet's radius.",
                args: &[
                    FEATURE,
                    ArgSpec {
                        name: "size",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres.",
                    },
                ],
                build: |args| {
                    Ok(Box::new(EditBlend::new(
                        FeatureId(id(args, "feature")?),
                        number(args, "size")?,
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
                description: "Create an empty sketch on an origin plane or on a flat face of the part.",
                args: &[
                    ArgSpec {
                        name: "plane",
                        kind: ArgKind::Plane,
                        required: false,
                        description: "xy, xz or yz. Give this or face.",
                    },
                    ArgSpec {
                        name: "face",
                        kind: ArgKind::Text,
                        required: false,
                        description: "A flat face's persistent name, such as f0/end/4-5-6-7. Give this or plane.",
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
                    let plane = match (args.contains_key("plane"), args.contains_key("face")) {
                        (true, false) => plane(args, "plane")?,
                        (false, true) => Plane::Face(text(args, "face")?),
                        _ => return Err(String::from("give either \"plane\" or \"face\"")),
                    };
                    Ok(Box::new(CreateSketch::new(&name, plane)))
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

const HOLE_ARGS: [ArgSpec; 8] = [
    ArgSpec {
        name: "diameter",
        kind: ArgKind::Number,
        required: false,
        description: "Drill diameter in mm. Defaults to the tap drill when a thread is given.",
    },
    ArgSpec {
        name: "depth",
        kind: ArgKind::Depth,
        required: true,
        description: "Millimetres to the end of the full diameter, or \"through\".",
    },
    ArgSpec {
        name: "tip_angle",
        kind: ArgKind::Number,
        required: false,
        description: "Drill point angle in degrees, 118 by default; 180 is flat.",
    },
    ArgSpec {
        name: "counterbore_diameter",
        kind: ArgKind::Number,
        required: false,
        description: "Counterbore diameter in mm; give with counterbore_depth.",
    },
    ArgSpec {
        name: "counterbore_depth",
        kind: ArgKind::Number,
        required: false,
        description: "Counterbore depth in mm.",
    },
    ArgSpec {
        name: "thread",
        kind: ArgKind::Text,
        required: false,
        description: "Thread size, such as M6 or M6x1.0.",
    },
    ArgSpec {
        name: "thread_depth",
        kind: ArgKind::Number,
        required: false,
        description: "Full-thread depth in mm; defaults to the hole depth.",
    },
    ArgSpec {
        name: "fit",
        kind: ArgKind::Text,
        required: false,
        description: "A fit such as H7, for CAM to finish to.",
    },
];

const ADD_HOLE_ARGS: &[ArgSpec] = &[
    SKETCH,
    ArgSpec {
        name: "points",
        kind: ArgKind::Ids,
        required: false,
        description: "Point ids; by default every free-standing point of the sketch.",
    },
    ArgSpec {
        name: "name",
        kind: ArgKind::Text,
        required: false,
        description: "Defaults to the next HoleN.",
    },
    HOLE_ARGS[0],
    HOLE_ARGS[1],
    HOLE_ARGS[2],
    HOLE_ARGS[3],
    HOLE_ARGS[4],
    HOLE_ARGS[5],
    HOLE_ARGS[6],
    HOLE_ARGS[7],
];

const EDIT_HOLE_ARGS: &[ArgSpec] = &[
    FEATURE,
    HOLE_ARGS[0],
    HOLE_ARGS[1],
    HOLE_ARGS[2],
    HOLE_ARGS[3],
    HOLE_ARGS[4],
    HOLE_ARGS[5],
    HOLE_ARGS[6],
    HOLE_ARGS[7],
];

fn optional_number(args: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    if args.contains_key(key) {
        number(args, key).map(Some)
    } else {
        Ok(None)
    }
}

/// Reads a hole's size and type from command arguments.
fn hole_spec(args: &Map<String, Value>) -> Result<HoleSpec, String> {
    let depth = match args.get("depth") {
        Some(Value::String(s)) if s == "through" => HoleDepth::Through,
        Some(_) => HoleDepth::Blind(
            number(args, "depth")
                .map_err(|_| String::from("\"depth\" must be a number of mm or \"through\""))?,
        ),
        None => return Err(String::from("missing argument \"depth\"")),
    };
    let thread_size = if args.contains_key("thread") {
        Some(text(args, "thread")?)
    } else {
        None
    };
    let known = thread_size
        .as_deref()
        .map(|t| {
            cndt_model::threads::thread(t).ok_or_else(|| {
                format!("unknown thread {t:?}; use a metric coarse size such as M6x1.0")
            })
        })
        .transpose()?;
    let diameter = match (optional_number(args, "diameter")?, known) {
        (Some(d), _) => d,
        (None, Some(t)) => t.tap_drill(),
        (None, None) => {
            return Err(String::from(
                "give \"diameter\", or a \"thread\" to use its tap drill",
            ));
        }
    };
    let thread = match known {
        Some(t) => {
            let depth = match (optional_number(args, "thread_depth")?, depth) {
                (Some(d), _) | (None, HoleDepth::Blind(d)) => d,
                (None, HoleDepth::Through) => {
                    return Err(String::from(
                        "give \"thread_depth\" for a through tapped hole",
                    ));
                }
            };
            Some(Thread {
                size: t.designation.to_owned(),
                depth,
            })
        }
        None => None,
    };
    let counterbore = match (
        optional_number(args, "counterbore_diameter")?,
        optional_number(args, "counterbore_depth")?,
    ) {
        (Some(diameter), Some(depth)) => Some(Counterbore { diameter, depth }),
        (None, None) => None,
        _ => {
            return Err(String::from(
                "give both \"counterbore_diameter\" and \"counterbore_depth\"",
            ));
        }
    };
    let fit = if args.contains_key("fit") {
        Some(text(args, "fit")?)
    } else {
        None
    };
    Ok(HoleSpec {
        diameter,
        depth,
        tip_angle: optional_number(args, "tip_angle")?.unwrap_or(118.0),
        counterbore,
        thread,
        fit,
    })
}

fn ids(args: &Map<String, Value>, key: &str) -> Result<Vec<EntityId>, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or_default())
        .map_err(|_| format!("{key:?} must be a list of ids, like [3, 8]"))
}

const BLEND_TARGETS: [ArgSpec; 2] = [
    ArgSpec {
        name: "faces",
        kind: ArgKind::Faces,
        required: false,
        description: "Faces whose every edge is blended.",
    },
    ArgSpec {
        name: "edges",
        kind: ArgKind::Edges,
        required: false,
        description: "Single edges, each named by its two faces.",
    },
];

/// Reads the edges a chamfer or fillet blends.
fn edge_targets(args: &Map<String, Value>) -> Result<Vec<EdgeTarget>, String> {
    let mut targets = Vec::new();
    if args.contains_key("faces") {
        let faces: Vec<String> =
            serde_json::from_value(args.get("faces").cloned().unwrap_or_default())
                .map_err(|_| String::from("\"faces\" must be a list of face names"))?;
        targets.extend(faces.into_iter().map(EdgeTarget::Face));
    }
    if args.contains_key("edges") {
        let edges: Vec<(String, String)> =
            serde_json::from_value(args.get("edges").cloned().unwrap_or_default())
                .map_err(|_| String::from("\"edges\" must be a list of face-name pairs"))?;
        targets.extend(edges.into_iter().map(|(a, b)| EdgeTarget::Between(a, b)));
    }
    if targets.is_empty() {
        return Err(String::from("give \"faces\" or \"edges\" to blend"));
    }
    Ok(targets)
}

fn blend(
    args: &Map<String, Value>,
    kind: BlendKind,
    size: &str,
) -> Result<Box<dyn Command>, String> {
    let name = if args.contains_key("name") {
        Some(text(args, "name")?)
    } else {
        None
    };
    Ok(Box::new(AddBlend::new(
        kind,
        edge_targets(args)?,
        number(args, size)?,
        name,
    )))
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
            ("sketch.create", json!({}), "either"),
            (
                "feature.chamfer",
                json!({"distance": 1}),
                "give \"faces\" or \"edges\"",
            ),
            (
                "feature.fillet",
                json!({"edges": [["a"]], "radius": 1}),
                "face-name pairs",
            ),
            (
                "feature.hole",
                json!({"sketch": 0, "depth": 5}),
                "give \"diameter\"",
            ),
            (
                "feature.hole",
                json!({"sketch": 0, "diameter": 5, "depth": "deep"}),
                "or \"through\"",
            ),
            (
                "feature.hole",
                json!({"sketch": 0, "thread": "M7", "depth": 5}),
                "unknown thread",
            ),
            (
                "feature.hole",
                json!({"sketch": 0, "diameter": 5, "depth": 9, "counterbore_depth": 2}),
                "both",
            ),
            (
                "feature.hole",
                json!({"sketch": 0, "thread": "M6", "depth": "through"}),
                "thread_depth",
            ),
            (
                "sketch.create",
                json!({"plane": "xy", "face": "f0/end/1"}),
                "either",
            ),
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
            (
                "feature.chamfer",
                json!({"faces": ["f0/end/4-5-6-7"], "distance": 0.5}),
            ),
            (
                "feature.fillet",
                json!({"edges": [["f0/side/e4", "f0/side/e5"]], "radius": 3}),
            ),
            ("feature.edit_blend", json!({"feature": 3, "size": 1})),
            (
                "feature.hole",
                json!({"sketch": 0, "thread": "M6", "depth": 15}),
            ),
            (
                "feature.edit_hole",
                json!({"feature": 0, "diameter": 5.5, "depth": "through", "counterbore_diameter": 9.5, "counterbore_depth": 5.4}),
            ),
            ("sketch.create", json!({"face": "f0/end/4-5-6-7"})),
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
