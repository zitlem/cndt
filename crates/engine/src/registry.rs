// SPDX-License-Identifier: AGPL-3.0-or-later
//! Builds commands by name from JSON arguments, and describes each command's
//! arguments so front ends can generate forms and validate input.

use std::collections::BTreeMap;

use cndt_model::sketch::{Constraint, ConstraintId, EntityId, Point2};
use cndt_model::{Parameter, Plane, ProjectedShape, SketchId, Unit};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    AddBlend, AddExtrude, AddHole, AddImport, AddRevolve, AddShell, BlendKind, Command,
    CommandError, CreateSketch, DeleteFeature, DeleteSketch, EditBlend, EditCam, EditExtrude,
    EditHole, EditRevolve, EditShell, EditSketch, Grid, MoveFeature, PointRef, RemoveParameter,
    RenameDocument, SetCircularPattern, SetMirror, SetParameter, SetPattern, SketchEdit,
    SuppressFeature,
};
use crate::{AddWorkGeometry, DeleteWorkGeometry, ProjectEdge};
use cndt_model::sketch::ProfileKey;
use cndt_model::{Counterbore, EdgeTarget, HoleDepth, HoleSpec, MirrorPlane, Thread};
use cndt_model::{FeatureId, Operation, RevolveAxis};
use cndt_model::{OriginPlane, Reference, ReferenceId};

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
    /// A list of CAM operations as the document stores them, such as
    /// `[{"type": "face"}, {"type": "pocket", "floor": "f1/end/e0"}]`.
    Operations,
    /// A list of curves in sketch coordinates, such as
    /// `[{"type": "line", "start": [0, 0], "end": [10, 0]}]`; arcs have
    /// `center`, `start` and `end` (counter-clockwise), circles `center`
    /// and `radius`.
    Curves,
    /// A list of sketch points, each a position `{"x": 1, "y": 2}` or the id
    /// of an existing point, such as `[{"x": 0, "y": 0}, 4, {"x": 9, "y": 2}]`.
    Points,
    /// What a projected edge looks like in the sketch plane, as the API
    /// computes it: `{"type": "line", "start": {"x": 0, "y": 0}, "end": ...}`.
    /// Not typed by hand.
    Shape,
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
                    let (distance, expression) = number_or_expression(args, "distance")?;
                    Ok(Box::new(
                        AddExtrude::new(sketch_id(args)?, profiles, distance, operation, name)
                            .with_expression(expression),
                    ))
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
                    let (distance, expression) = if args.contains_key("distance") {
                        let (d, e) = number_or_expression(args, "distance")?;
                        (Some(d), e)
                    } else {
                        (None, None)
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
                    Ok(Box::new(
                        EditExtrude::new(FeatureId(id(args, "feature")?), distance, op, profiles)
                            .with_expression(expression),
                    ))
                },
            },
            CommandSpec {
                name: "feature.revolve",
                description: "Turn sketch profiles about an axis in their plane into a solid, or cut them from it. The profile may touch the axis but not cross it.",
                args: REVOLVE_ARGS,
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
                    let (angle, expression) = if args.contains_key("angle") {
                        number_or_expression(args, "angle")?
                    } else {
                        (360.0, None)
                    };
                    Ok(Box::new(
                        AddRevolve::new(
                            sketch_id(args)?,
                            profiles,
                            revolve_axis(args)?.unwrap_or(RevolveAxis::SketchY),
                            angle,
                            operation,
                            name,
                        )
                        .with_expression(expression),
                    ))
                },
            },
            CommandSpec {
                name: "feature.edit_revolve",
                description: "Change a revolve's angle, axis, operation or profiles.",
                args: EDIT_REVOLVE_ARGS,
                build: |args| {
                    let (angle, expression) = if args.contains_key("angle") {
                        let (a, e) = number_or_expression(args, "angle")?;
                        (Some(a), e)
                    } else {
                        (None, None)
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
                    Ok(Box::new(
                        EditRevolve::new(
                            FeatureId(id(args, "feature")?),
                            angle,
                            revolve_axis(args)?,
                            op,
                            profiles,
                        )
                        .with_expression(expression),
                    ))
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
                name: "feature.import",
                description: "Add a solid from a STEP or IGES file, kept in the project.",
                args: &[
                    ArgSpec {
                        name: "source",
                        kind: ArgKind::Text,
                        required: true,
                        description: "The file's name, shown in the timeline.",
                    },
                    ArgSpec {
                        name: "content",
                        kind: ArgKind::Text,
                        required: true,
                        description: "The STEP or IGES file's text.",
                    },
                    ArgSpec {
                        name: "operation",
                        kind: ArgKind::Operation,
                        required: false,
                        description: "join (the default) or cut.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next ImportN.",
                    },
                ],
                build: |args| {
                    let operation = if args.contains_key("operation") {
                        operation(args)?
                    } else {
                        Operation::Join
                    };
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(AddImport::new(
                        text(args, "source")?,
                        text(args, "content")?,
                        operation,
                        name,
                    )))
                },
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
                name: "feature.shell",
                description: "Hollow the part inward, leaving the given faces open.",
                args: &[
                    ArgSpec {
                        name: "faces",
                        kind: ArgKind::Faces,
                        required: true,
                        description: "The faces left open, by name.",
                    },
                    ArgSpec {
                        name: "thickness",
                        kind: ArgKind::Number,
                        required: true,
                        description: "The wall's thickness in millimetres.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next ShellN.",
                    },
                ],
                build: |args| {
                    let faces: Vec<String> =
                        serde_json::from_value(args.get("faces").cloned().unwrap_or_default())
                            .map_err(|_| String::from("\"faces\" must be a list of face names"))?;
                    if faces.is_empty() {
                        return Err(String::from("give the \"faces\" to leave open"));
                    }
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(AddShell::new(
                        faces,
                        number(args, "thickness")?,
                        name,
                    )))
                },
            },
            CommandSpec {
                name: "feature.edit_shell",
                description: "Change a shell's wall thickness.",
                args: &[
                    FEATURE,
                    ArgSpec {
                        name: "thickness",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres.",
                    },
                ],
                build: |args| {
                    Ok(Box::new(EditShell::new(
                        FeatureId(id(args, "feature")?),
                        number(args, "thickness")?,
                    )))
                },
            },
            CommandSpec {
                name: "feature.pattern",
                description: "Repeat an extrude or hole in a grid along its sketch's X and Y.",
                args: &[
                    ArgSpec {
                        name: "feature",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The extrude or hole to repeat.",
                    },
                    GRID_ARGS[0],
                    GRID_ARGS[1],
                    GRID_ARGS[2],
                    GRID_ARGS[3],
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next PatternN.",
                    },
                ],
                build: |args| {
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(SetPattern::add(
                        FeatureId(id(args, "feature")?),
                        grid(args)?,
                        name,
                    )))
                },
            },
            CommandSpec {
                name: "feature.edit_pattern",
                description: "Change a pattern's grid.",
                args: &[
                    FEATURE,
                    GRID_ARGS[0],
                    GRID_ARGS[1],
                    GRID_ARGS[2],
                    GRID_ARGS[3],
                ],
                build: |args| {
                    Ok(Box::new(SetPattern::edit(
                        FeatureId(id(args, "feature")?),
                        grid(args)?,
                    )))
                },
            },
            CommandSpec {
                name: "feature.circular_pattern",
                description: "Repeat an extrude, hole or revolve turned about its sketch's normal through a point.",
                args: &[
                    ArgSpec {
                        name: "feature",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The extrude, hole or revolve to repeat.",
                    },
                    CIRCULAR_ARGS[0],
                    CIRCULAR_ARGS[1],
                    CIRCULAR_ARGS[2],
                    CIRCULAR_ARGS[3],
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next CircularPatternN.",
                    },
                ],
                build: |args| {
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(SetCircularPattern::add(
                        FeatureId(id(args, "feature")?),
                        id(args, "count")?,
                        circular_angle(args)?,
                        circular_center(args)?,
                        axis_work(args)?,
                        name,
                    )))
                },
            },
            CommandSpec {
                name: "feature.edit_circular_pattern",
                description: "Change a circular pattern's count, angle and centre.",
                args: &[
                    FEATURE,
                    CIRCULAR_ARGS[0],
                    CIRCULAR_ARGS[1],
                    CIRCULAR_ARGS[2],
                    CIRCULAR_ARGS[3],
                ],
                build: |args| {
                    Ok(Box::new(SetCircularPattern::edit(
                        FeatureId(id(args, "feature")?),
                        id(args, "count")?,
                        circular_angle(args)?,
                        circular_center(args)?,
                        axis_work(args)?,
                    )))
                },
            },
            CommandSpec {
                name: "feature.mirror",
                description: "Add the mirror image of an extrude, hole or revolve.",
                args: &[
                    ArgSpec {
                        name: "feature",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The extrude, hole or revolve to mirror.",
                    },
                    MIRROR_PLANE,
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next MirrorN.",
                    },
                ],
                build: |args| {
                    let name = if args.contains_key("name") {
                        Some(text(args, "name")?)
                    } else {
                        None
                    };
                    Ok(Box::new(SetMirror::add(
                        FeatureId(id(args, "feature")?),
                        mirror_plane(args)?,
                        name,
                    )))
                },
            },
            CommandSpec {
                name: "feature.edit_mirror",
                description: "Change the plane a mirror reflects in.",
                args: &[FEATURE, MIRROR_PLANE],
                build: |args| {
                    Ok(Box::new(SetMirror::edit(
                        FeatureId(id(args, "feature")?),
                        mirror_plane(args)?,
                    )))
                },
            },
            CommandSpec {
                name: "cam.set_stock",
                description: "Set the raw stock: the part's box plus allowances.",
                args: &[
                    ArgSpec {
                        name: "side",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres on each of the four sides.",
                    },
                    ArgSpec {
                        name: "top",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres above the part.",
                    },
                    ArgSpec {
                        name: "bottom",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres below the part.",
                    },
                ],
                build: |args| {
                    let stock = cndt_model::cam::Stock {
                        side: number(args, "side")?,
                        top: number(args, "top")?,
                        bottom: number(args, "bottom")?,
                    };
                    cam("cam.set_stock", cndt_model::cam::CamEdit::SetStock(stock))
                },
            },
            CommandSpec {
                name: "cam.add_setup",
                description: "Add a setup: which side faces the spindle, and the work offset. The first setup brings in the default tool library.",
                args: &[
                    ArgSpec {
                        name: "side",
                        kind: ArgKind::Text,
                        required: true,
                        description: "top; bottom for the part turned over; front, back, left or right for a side turned a quarter to the spindle.",
                    },
                    ArgSpec {
                        name: "flip_about",
                        kind: ArgKind::Text,
                        required: false,
                        description: "For the bottom: x (the default) or y, the model axis the part is turned about. A horizontal's B table turns it about the upright axis.",
                    },
                    ArgSpec {
                        name: "index",
                        kind: ArgKind::Number,
                        required: false,
                        description: "B angle in degrees (0 to under 360) when the rotary table, not the operator, turns this side to the spindle.",
                    },
                    ArgSpec {
                        name: "angle",
                        kind: ArgKind::Number,
                        required: false,
                        description: "For side turned: degrees the part is turned about model Y to bring the face to the spindle (0 the top, 90 the right).",
                    },
                    ArgSpec {
                        name: "work_offset",
                        kind: ArgKind::Id,
                        required: false,
                        description: "1 (G54, the default) to 6 (G59).",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to Setup N (side).",
                    },
                ],
                build: |args| {
                    let work_offset = if args.contains_key("work_offset") {
                        u8::try_from(id(args, "work_offset")?)
                            .map_err(|_| String::from("\"work_offset\" must be 1 to 6"))?
                    } else {
                        1
                    };
                    let name = if args.contains_key("name") {
                        text(args, "name")?
                    } else {
                        String::new()
                    };
                    cam(
                        "cam.add_setup",
                        cndt_model::cam::CamEdit::AddSetup {
                            name,
                            side: side(args)?,
                            work_offset,
                            flip_about: if args.contains_key("flip_about") {
                                match text(args, "flip_about")?.as_str() {
                                    "x" => cndt_model::cam::FlipAxis::X,
                                    "y" => cndt_model::cam::FlipAxis::Y,
                                    other => {
                                        return Err(format!(
                                            "unknown flip axis {other:?}; use x or y"
                                        ));
                                    }
                                }
                            } else {
                                cndt_model::cam::FlipAxis::X
                            },
                            index: optional_number(args, "index")?,
                            angle: optional_number(args, "angle")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "cam.remove_setup",
                description: "Delete a setup and its operations.",
                args: &[ArgSpec {
                    name: "setup",
                    kind: ArgKind::Id,
                    required: true,
                    description: "The setup's id.",
                }],
                build: |args| {
                    cam(
                        "cam.remove_setup",
                        cndt_model::cam::CamEdit::RemoveSetup(id(args, "setup")?),
                    )
                },
            },
            CommandSpec {
                name: "cam.add_operation",
                description: "Add an operation at the end of a setup. Kinds: face (face the stock down to the part), contour (follow a face's outline outside it), pocket (clear down to a floor face), holes (spot, drill, tap and ream every hole facing the spindle).",
                args: &[
                    ArgSpec {
                        name: "setup",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The setup's id.",
                    },
                    ArgSpec {
                        name: "type",
                        kind: ArgKind::Text,
                        required: true,
                        description: "face, contour, pocket, holes or chamfer",
                    },
                    ArgSpec {
                        name: "width",
                        kind: ArgKind::Number,
                        required: false,
                        description: "For chamfer: the chamfer's width in mm, as modelled.",
                    },
                    ArgSpec {
                        name: "floor",
                        kind: ArgKind::Text,
                        required: false,
                        description: "For pocket: the pocket's flat floor face.",
                    },
                    ArgSpec {
                        name: "finish",
                        kind: ArgKind::Number,
                        required: false,
                        description: "For pocket: mm the roughing leaves on the walls, taken off by one full-depth finishing lap.",
                    },
                    ArgSpec {
                        name: "tool",
                        kind: ArgKind::Id,
                        required: false,
                        description: "For face and contour: the tool number; by default the first face mill or end mill.",
                    },
                    ArgSpec {
                        name: "face",
                        kind: ArgKind::Text,
                        required: false,
                        description: "For contour: the flat face whose outline is followed; for chamfer: the face whose edges are chamfered.",
                    },
                    ArgSpec {
                        name: "depth",
                        kind: ArgKind::Number,
                        required: false,
                        description: "For contour: millimetres below Z0; by default the face's depth.",
                    },
                ],
                build: |args| {
                    let operation = match text(args, "type")?.as_str() {
                        "holes" => cndt_model::cam::OperationDef::Holes,
                        "chamfer" => cndt_model::cam::OperationDef::Chamfer {
                            face: text(args, "face")?,
                            width: number(args, "width")?,
                            tool: if args.contains_key("tool") {
                                Some(id(args, "tool")?)
                            } else {
                                None
                            },
                        },
                        "pocket" => cndt_model::cam::OperationDef::Pocket {
                            floor: text(args, "floor")?,
                            tool: if args.contains_key("tool") {
                                Some(id(args, "tool")?)
                            } else {
                                None
                            },
                            finish: optional_number(args, "finish")?,
                        },
                        "contour" => cndt_model::cam::OperationDef::Contour {
                            face: text(args, "face")?,
                            tool: if args.contains_key("tool") {
                                Some(id(args, "tool")?)
                            } else {
                                None
                            },
                            depth: optional_number(args, "depth")?,
                        },
                        "face" => cndt_model::cam::OperationDef::Face {
                            tool: if args.contains_key("tool") {
                                Some(id(args, "tool")?)
                            } else {
                                None
                            },
                        },
                        other => {
                            return Err(format!(
                                "unknown operation {other:?}; use face, contour or holes"
                            ));
                        }
                    };
                    cam(
                        "cam.add_operation",
                        cndt_model::cam::CamEdit::AddOperation {
                            setup: id(args, "setup")?,
                            operation,
                        },
                    )
                },
            },
            CommandSpec {
                name: "cam.add_operations",
                description: "Add several operations at the end of a setup as one step, such as those the detect request suggests.",
                args: &[
                    ArgSpec {
                        name: "setup",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The setup's id.",
                    },
                    ArgSpec {
                        name: "operations",
                        kind: ArgKind::Operations,
                        required: true,
                        description: "The operations, as the document stores them.",
                    },
                ],
                build: |args| {
                    let operations = args
                        .get("operations")
                        .cloned()
                        .ok_or_else(|| String::from("missing argument operations"))
                        .and_then(|value| {
                            serde_json::from_value::<Vec<cndt_model::cam::OperationDef>>(value)
                                .map_err(|e| format!("operations: {e}"))
                        })?;
                    cam(
                        "cam.add_operations",
                        cndt_model::cam::CamEdit::AddOperations {
                            setup: id(args, "setup")?,
                            operations,
                        },
                    )
                },
            },
            CommandSpec {
                name: "cam.remove_operation",
                description: "Delete an operation from a setup.",
                args: &[
                    ArgSpec {
                        name: "setup",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The setup's id.",
                    },
                    ArgSpec {
                        name: "operation",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The operation's id.",
                    },
                ],
                build: |args| {
                    cam(
                        "cam.remove_operation",
                        cndt_model::cam::CamEdit::RemoveOperation {
                            setup: id(args, "setup")?,
                            operation: id(args, "operation")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "cam.set_tool",
                description: "Add a tool to the library, or replace one.",
                args: &[
                    ArgSpec {
                        name: "number",
                        kind: ArgKind::Id,
                        required: true,
                        description: "Pocket and length offset number, 1 to 999.",
                    },
                    ArgSpec {
                        name: "kind",
                        kind: ArgKind::Text,
                        required: true,
                        description: "spot_drill, drill, tap, reamer, end_mill, face_mill or chamfer_mill.",
                    },
                    ArgSpec {
                        name: "thread",
                        kind: ArgKind::Text,
                        required: false,
                        description: "For a tap: its thread, such as M6x1.0.",
                    },
                    ArgSpec {
                        name: "diameter",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres.",
                    },
                    ArgSpec {
                        name: "rpm",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Spindle speed.",
                    },
                    ArgSpec {
                        name: "feed",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres per minute.",
                    },
                    ArgSpec {
                        name: "description",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Shown in the program.",
                    },
                    ArgSpec {
                        name: "flute_length",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Measured assembly, mm: how far up from the tip it cuts. Give all five assembly values, or none for typical sizes.",
                    },
                    ArgSpec {
                        name: "stickout",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Measured assembly, mm: tip to holder face.",
                    },
                    ArgSpec {
                        name: "shank_diameter",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Measured assembly, mm.",
                    },
                    ArgSpec {
                        name: "holder_diameter",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Measured assembly, mm.",
                    },
                    ArgSpec {
                        name: "holder_length",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Measured assembly, mm: from the holder face towards the spindle.",
                    },
                ],
                build: |args| {
                    const ASSEMBLY: [&str; 5] = [
                        "flute_length",
                        "stickout",
                        "shank_diameter",
                        "holder_diameter",
                        "holder_length",
                    ];
                    let given = ASSEMBLY.iter().filter(|k| args.contains_key(**k)).count();
                    let assembly = match given {
                        0 => None,
                        5 => Some(cndt_model::cam::Assembly {
                            flute_length: number(args, "flute_length")?,
                            stickout: number(args, "stickout")?,
                            shank_diameter: number(args, "shank_diameter")?,
                            holder_diameter: number(args, "holder_diameter")?,
                            holder_length: number(args, "holder_length")?,
                        }),
                        _ => {
                            return Err(String::from(
                                "give all five assembly values (flute_length, stickout, shank_diameter, holder_diameter, holder_length) or none",
                            ));
                        }
                    };
                    let tool = cndt_model::cam::Tool {
                        kind: tool_kind(args)?,
                        diameter: number(args, "diameter")?,
                        rpm: number(args, "rpm")?,
                        feed: number(args, "feed")?,
                        description: if args.contains_key("description") {
                            text(args, "description")?
                        } else {
                            String::new()
                        },
                        assembly,
                    };
                    cam(
                        "cam.set_tool",
                        cndt_model::cam::CamEdit::SetTool {
                            number: id(args, "number")?,
                            tool,
                        },
                    )
                },
            },
            CommandSpec {
                name: "cam.set_vise",
                description: "Hold a setup's stock in a vise, so the simulation checks no tool touches its jaws; without grip, remove it.",
                args: &[
                    ArgSpec {
                        name: "setup",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The setup.",
                    },
                    ArgSpec {
                        name: "grip",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Millimetres of the stock held in the jaws, from its bottom up. Leave out to remove the vise.",
                    },
                    ArgSpec {
                        name: "along",
                        kind: ArgKind::Text,
                        required: false,
                        description: "x (the default: jaws on the front and back) or y.",
                    },
                    ArgSpec {
                        name: "jaw",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Each jaw's thickness in millimetres; 25 by default.",
                    },
                    ArgSpec {
                        name: "body",
                        kind: ArgKind::Number,
                        required: false,
                        description: "How far below the stock's bottom the vise's fixed body (its floor) stands, in millimetres. Leave out for none.",
                    },
                ],
                build: |args| {
                    use cndt_model::cam::{CamEdit, DEFAULT_JAW, JawAxis, Vise};
                    let setup = id(args, "setup")?;
                    let vise = if args.contains_key("grip") {
                        let along = if args.contains_key("along") {
                            match text(args, "along")?.as_str() {
                                "x" => JawAxis::X,
                                "y" => JawAxis::Y,
                                other => {
                                    return Err(format!("unknown jaw axis {other:?}; use x or y"));
                                }
                            }
                        } else {
                            JawAxis::X
                        };
                        Some(Vise {
                            grip: number(args, "grip")?,
                            along,
                            jaw: optional_number(args, "jaw")?.unwrap_or(DEFAULT_JAW),
                            body: optional_number(args, "body")?,
                        })
                    } else {
                        None
                    };
                    Ok(Box::new(EditCam::new(
                        "cam.set_vise",
                        CamEdit::SetVise { setup, vise },
                    )))
                },
            },
            CommandSpec {
                name: "cam.remove_tool",
                description: "Delete a tool from the library.",
                args: &[ArgSpec {
                    name: "number",
                    kind: ArgKind::Id,
                    required: true,
                    description: "The tool's number.",
                }],
                build: |args| {
                    cam(
                        "cam.remove_tool",
                        cndt_model::cam::CamEdit::RemoveTool(id(args, "number")?),
                    )
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
                name: "feature.suppress",
                description: "Leave a feature out of the part, or put it back. Features that repeat a suppressed one fail with the reason.",
                args: &[
                    FEATURE,
                    ArgSpec {
                        name: "suppressed",
                        kind: ArgKind::Boolean,
                        required: false,
                        description: "true to suppress (the default), false to put it back.",
                    },
                ],
                build: |args| {
                    let suppressed = if args.contains_key("suppressed") {
                        boolean(args, "suppressed")?
                    } else {
                        true
                    };
                    Ok(Box::new(SuppressFeature::new(
                        FeatureId(id(args, "feature")?),
                        suppressed,
                    )))
                },
            },
            CommandSpec {
                name: "feature.move",
                description: "Move a feature to another position of the timeline. Refused when it would come before the feature it repeats or the face it is sketched on.",
                args: &[
                    FEATURE,
                    ArgSpec {
                        name: "position",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The new position in the timeline, counting from 0.",
                    },
                ],
                build: |args| {
                    let to = usize::try_from(id(args, "position")?)
                        .map_err(|_| String::from("position is too large"))?;
                    Ok(Box::new(MoveFeature::new(
                        FeatureId(id(args, "feature")?),
                        to,
                    )))
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
                        description: "xy, xz or yz. Give this, face or work_plane.",
                    },
                    ArgSpec {
                        name: "work_plane",
                        kind: ArgKind::Id,
                        required: false,
                        description: "The id of a work plane (reference.add_work_plane). Give this, plane or face.",
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
                    let plane = match (
                        args.contains_key("plane"),
                        args.contains_key("face"),
                        args.contains_key("work_plane"),
                    ) {
                        (true, false, false) => plane(args, "plane")?,
                        (false, true, false) => Plane::Face(text(args, "face")?),
                        (false, false, true) => Plane::Work(ReferenceId(id(args, "work_plane")?)),
                        _ => {
                            return Err(String::from(
                                "give either \"plane\", \"face\" or \"work_plane\", and only one",
                            ));
                        }
                    };
                    Ok(Box::new(CreateSketch::new(&name, plane)))
                },
            },
            CommandSpec {
                name: "reference.add_work_plane",
                description: "Add a work plane: an origin plane moved along its normal. Sketches can lie on it.",
                args: &[
                    ArgSpec {
                        name: "plane",
                        kind: ArgKind::Plane,
                        required: true,
                        description: "The origin plane to start from: xy, xz or yz.",
                    },
                    ArgSpec {
                        name: "offset",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Millimetres along the plane's normal; negative goes the other way.",
                    },
                    WORK_NAME,
                ],
                build: |args| {
                    let given = text(args, "plane")?;
                    let base = OriginPlane::from_name(&given)
                        .ok_or_else(|| format!("unknown plane {given:?}; use xy, xz or yz"))?;
                    Ok(Box::new(AddWorkGeometry::new(Reference::WorkPlane {
                        name: optional_text(args, "name")?,
                        base,
                        offset: number(args, "offset")?,
                    })))
                },
            },
            CommandSpec {
                name: "reference.add_work_axis",
                description: "Add a work axis: a line through a point in a direction, in model coordinates. A revolve or circular pattern can turn about it.",
                args: &[
                    WORK_X,
                    WORK_Y,
                    WORK_Z,
                    ArgSpec {
                        name: "dx",
                        kind: ArgKind::Number,
                        required: false,
                        description: "X of the direction. 0 by default; the direction must not be all zero.",
                    },
                    ArgSpec {
                        name: "dy",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Y of the direction. 0 by default.",
                    },
                    ArgSpec {
                        name: "dz",
                        kind: ArgKind::Number,
                        required: false,
                        description: "Z of the direction. 0 by default.",
                    },
                    WORK_NAME,
                ],
                build: |args| {
                    Ok(Box::new(AddWorkGeometry::new(Reference::WorkAxis {
                        name: optional_text(args, "name")?,
                        through: triple(args, "")?,
                        direction: triple(args, "d")?,
                    })))
                },
            },
            CommandSpec {
                name: "reference.add_work_point",
                description: "Add a work point at a position in model coordinates.",
                args: &[WORK_X, WORK_Y, WORK_Z, WORK_NAME],
                build: |args| {
                    Ok(Box::new(AddWorkGeometry::new(Reference::WorkPoint {
                        name: optional_text(args, "name")?,
                        position: triple(args, "")?,
                    })))
                },
            },
            CommandSpec {
                name: "reference.delete",
                description: "Delete a work plane, axis or point that no sketch or feature uses.",
                args: &[ArgSpec {
                    name: "reference",
                    kind: ArgKind::Id,
                    required: true,
                    description: "The reference geometry's id.",
                }],
                build: |args| {
                    Ok(Box::new(DeleteWorkGeometry::new(ReferenceId(id(
                        args,
                        "reference",
                    )?))))
                },
            },
            CommandSpec {
                name: "sketch.import",
                description: "Create a sketch holding curves, such as those read from a DXF file, as one step. Ends that meet become one point.",
                args: &[
                    ArgSpec {
                        name: "curves",
                        kind: ArgKind::Curves,
                        required: true,
                        description: "The lines, arcs and circles.",
                    },
                    ArgSpec {
                        name: "plane",
                        kind: ArgKind::Plane,
                        required: false,
                        description: "xy (the default), xz or yz.",
                    },
                    ArgSpec {
                        name: "name",
                        kind: ArgKind::Text,
                        required: false,
                        description: "Defaults to the next SketchN.",
                    },
                ],
                build: |args| {
                    let curves = args
                        .get("curves")
                        .cloned()
                        .ok_or_else(|| String::from("missing argument curves"))
                        .and_then(|value| {
                            serde_json::from_value::<Vec<crate::ImportCurve>>(value)
                                .map_err(|e| format!("curves: {e}"))
                        })?;
                    let content = crate::sketch_from_curves(&curves)?;
                    let name = if args.contains_key("name") {
                        text(args, "name")?
                    } else {
                        String::new()
                    };
                    let plane = if args.contains_key("plane") {
                        plane(args, "plane")?
                    } else {
                        Plane::Xy
                    };
                    Ok(Box::new(CreateSketch::with_content(&name, plane, content)))
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
                name: "sketch.add_slot",
                description: "Add a slot: two arcs joined by two tangent lines, from the centres of its two ends and its width. Its width (as a radius) and the distance between the centres are dimensions.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "from",
                        kind: ArgKind::Point,
                        required: true,
                        description: "The centre of one end, in millimetres.",
                    },
                    ArgSpec {
                        name: "to",
                        kind: ArgKind::Point,
                        required: true,
                        description: "The centre of the other end.",
                    },
                    ArgSpec {
                        name: "width",
                        kind: ArgKind::Number,
                        required: true,
                        description: "Width across the slot, in millimetres.",
                    },
                ],
                build: |args| {
                    let (from, to) = (position(args, "from")?, position(args, "to")?);
                    edit(
                        args,
                        SketchEdit::AddSlot {
                            from,
                            to,
                            width: number(args, "width")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.add_spline",
                description: "Add a spline: a smooth curve through three or more points, each a new position or an existing point's id. Closed, it also returns from the last point to the first. Move a point and the curve follows; constrain the points as usual. It can be a profile, but cannot be trimmed or offset.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "through",
                        kind: ArgKind::Points,
                        required: true,
                        description: "At least three points, in order: positions or existing point ids.",
                    },
                    ArgSpec {
                        name: "closed",
                        kind: ArgKind::Boolean,
                        required: false,
                        description: "true to close the curve back on its first point (default false).",
                    },
                ],
                build: |args| {
                    let items = args
                        .get("through")
                        .and_then(Value::as_array)
                        .ok_or_else(|| String::from("\"through\" must be a list of points"))?;
                    let through = items
                        .iter()
                        .map(|item| {
                            let mut one = Map::new();
                            one.insert(String::from("point"), item.clone());
                            point_ref(&one, "point").map_err(|_| {
                                String::from(
                                    "\"through\" must hold positions like {\"x\": 1, \"y\": 2} or point ids",
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let closed = if args.contains_key("closed") {
                        boolean(args, "closed")?
                    } else {
                        false
                    };
                    edit(args, SketchEdit::AddSpline { through, closed })
                },
            },
            CommandSpec {
                name: "sketch.trim",
                description: "Trim a line, arc or circle: remove the stretch of it nearest a picked point, between the places where other curves cross it. Refused if nothing crosses it. Constraints on the trimmed entity are dropped.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entity",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The line, arc or circle to trim.",
                    },
                    ArgSpec {
                        name: "at",
                        kind: ArgKind::Point,
                        required: true,
                        description: "A point on the stretch to remove, in millimetres.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::Trim {
                            entity: EntityId(id(args, "entity")?),
                            at: position(args, "at")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.project_edge",
                description: "Project a model edge, named by the two faces that meet at it, into a sketch as a line, arc or circle with its points fixed. The sketch remembers the edge: if the part changes so the edge is lost or moves, features using the sketch fail and say so. Refused for an edge square to the sketch plane or one that does not project to a line, arc or circle.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "first",
                        kind: ArgKind::Text,
                        required: true,
                        description: "The name of one face at the edge.",
                    },
                    ArgSpec {
                        name: "second",
                        kind: ArgKind::Text,
                        required: true,
                        description: "The name of the other face.",
                    },
                    ArgSpec {
                        name: "shape",
                        kind: ArgKind::Shape,
                        required: false,
                        description: "Filled in by the API from the part; commands run without the API refuse.",
                    },
                ],
                build: |args| {
                    let shape = args.get("shape").cloned().ok_or_else(|| {
                        String::from(
                            "projecting an edge needs the geometry kernel; run it through the API",
                        )
                    })?;
                    let shape: ProjectedShape = serde_json::from_value(shape)
                        .map_err(|e| format!("shape is not a projected shape: {e}"))?;
                    Ok(Box::new(ProjectEdge::new(
                        sketch_id(args)?,
                        &text(args, "first")?,
                        &text(args, "second")?,
                        shape,
                    )))
                },
            },
            CommandSpec {
                name: "sketch.offset",
                description: "Offset a circle, or lines and arcs joined end to end into one chain, a distance toward a picked side. The copy is new geometry; the original stays. Corners are extended to meet. Refused if the copy would collapse.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entities",
                        kind: ArgKind::Ids,
                        required: true,
                        description: "One circle, or the lines and arcs of one chain.",
                    },
                    ArgSpec {
                        name: "distance",
                        kind: ArgKind::Number,
                        required: true,
                        description: "How far to offset, in millimetres; more than zero.",
                    },
                    ArgSpec {
                        name: "toward",
                        kind: ArgKind::Point,
                        required: true,
                        description: "A point on the side to offset toward, in millimetres.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::Offset {
                            entities: ids(args, "entities")?,
                            distance: number(args, "distance")?,
                            toward: position(args, "toward")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.mirror",
                description: "Copy lines, arcs, circles and points to the other side of a line. Each copied point is held symmetric to its original about the line, so editing one moves the other. Points on the line are shared. The originals stay.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entities",
                        kind: ArgKind::Ids,
                        required: true,
                        description: "The entities to copy.",
                    },
                    ArgSpec {
                        name: "axis",
                        kind: ArgKind::Id,
                        required: true,
                        description: "The line to mirror about.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::Mirror {
                            entities: ids(args, "entities")?,
                            axis: EntityId(id(args, "axis")?),
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.pattern_linear",
                description: "Repeat entities along a straight step. The count includes the original. Each copy is held equal to its original in length or radius.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entities",
                        kind: ArgKind::Ids,
                        required: true,
                        description: "The entities to repeat.",
                    },
                    ArgSpec {
                        name: "count",
                        kind: ArgKind::Id,
                        required: true,
                        description: "How many in all, the original included; 2 to 200.",
                    },
                    ArgSpec {
                        name: "step",
                        kind: ArgKind::Point,
                        required: true,
                        description: "The offset from one copy to the next, in millimetres.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::PatternLinear {
                            entities: ids(args, "entities")?,
                            count: id(args, "count")?,
                            step: position(args, "step")?,
                        },
                    )
                },
            },
            CommandSpec {
                name: "sketch.pattern_circular",
                description: "Repeat entities round a centre. The count includes the original. At 360 degrees the copies are spread round the full circle; less is the turn to the last copy. Each copy is held equal to its original in length or radius.",
                args: &[
                    SKETCH,
                    ArgSpec {
                        name: "entities",
                        kind: ArgKind::Ids,
                        required: true,
                        description: "The entities to repeat.",
                    },
                    ArgSpec {
                        name: "count",
                        kind: ArgKind::Id,
                        required: true,
                        description: "How many in all, the original included; 2 to 200.",
                    },
                    ArgSpec {
                        name: "center",
                        kind: ArgKind::Point,
                        required: true,
                        description: "The centre to turn about, in millimetres.",
                    },
                    ArgSpec {
                        name: "angle",
                        kind: ArgKind::Number,
                        required: false,
                        description: "The turn in degrees, more than 0 and at most 360. Default 360.",
                    },
                ],
                build: |args| {
                    edit(
                        args,
                        SketchEdit::PatternCircular {
                            entities: ids(args, "entities")?,
                            count: id(args, "count")?,
                            center: position(args, "center")?,
                            angle: optional_number(args, "angle")?.unwrap_or(360.0),
                        },
                    )
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
                        description: "Millimetres, or degrees for an angle; or an expression of the parameters, such as \"thickness / 2\".",
                    },
                ],
                build: |args| {
                    let constraint = ConstraintId(id(args, "constraint")?);
                    let edit_kind = match number_or_expression(args, "value")? {
                        (_, Some(expression)) => SketchEdit::SetDimensionExpression {
                            constraint,
                            expression,
                        },
                        (value, None) => SketchEdit::SetDimension { constraint, value },
                    };
                    edit(args, edit_kind)
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

/// A number, or an expression of the parameters such as `"thickness / 2"`
/// (evaluated when the command runs; the number is then a placeholder).
fn number_or_expression(
    args: &Map<String, Value>,
    key: &str,
) -> Result<(f64, Option<String>), String> {
    match args.get(key) {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            Ok((0.0, Some(text.trim().to_owned())))
        }
        _ => number(args, key).map(|n| (n, None)),
    }
}

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

const GRID_ARGS: [ArgSpec; 4] = [
    ArgSpec {
        name: "count_x",
        kind: ArgKind::Id,
        required: true,
        description: "Copies along the sketch's X, counting the original.",
    },
    ArgSpec {
        name: "spacing_x",
        kind: ArgKind::Number,
        required: false,
        description: "Millimetres between them (needed when count_x > 1).",
    },
    ArgSpec {
        name: "count_y",
        kind: ArgKind::Id,
        required: false,
        description: "Copies along the sketch's Y; 1 by default.",
    },
    ArgSpec {
        name: "spacing_y",
        kind: ArgKind::Number,
        required: false,
        description: "Millimetres between them.",
    },
];

const CIRCULAR_ARGS: [ArgSpec; 4] = [
    ArgSpec {
        name: "count",
        kind: ArgKind::Id,
        required: true,
        description: "Copies, counting the original; at least 2.",
    },
    ArgSpec {
        name: "angle",
        kind: ArgKind::Number,
        required: false,
        description: "Degrees the copies span, more than 0 up to 360. 360 by default.",
    },
    ArgSpec {
        name: "center",
        kind: ArgKind::Id,
        required: false,
        description: "A point of the source's sketch to turn about; the sketch origin by default.",
    },
    AXIS_WORK,
];

const AXIS_WORK: ArgSpec = ArgSpec {
    name: "axis_work",
    kind: ArgKind::Id,
    required: false,
    description: "The id of a work axis to turn about (reference.add_work_axis). For a revolve the axis must lie in the sketch's plane; instead of axis and axis_line. For a circular pattern, instead of center.",
};

fn axis_work(args: &Map<String, Value>) -> Result<Option<ReferenceId>, String> {
    if args.contains_key("axis_work") {
        id(args, "axis_work").map(|value| Some(ReferenceId(value)))
    } else {
        Ok(None)
    }
}

const MIRROR_PLANE: ArgSpec = ArgSpec {
    name: "plane",
    kind: ArgKind::Text,
    required: true,
    description: "sketch_x, sketch_y or sketch_z (through the sketch's origin, normal along its X, Y or Z), or model_x, model_y or model_z (through the model's origin).",
};

fn mirror_plane(args: &Map<String, Value>) -> Result<MirrorPlane, String> {
    let given = text(args, "plane")?;
    MirrorPlane::parse(&given).ok_or_else(|| {
        format!(
            "unknown mirror plane \"{given}\"; use sketch_x, sketch_y, sketch_z, model_x, model_y or model_z"
        )
    })
}

fn circular_angle(args: &Map<String, Value>) -> Result<f64, String> {
    Ok(optional_number(args, "angle")?.unwrap_or(360.0))
}

fn circular_center(args: &Map<String, Value>) -> Result<Option<EntityId>, String> {
    if args.contains_key("center") {
        id(args, "center").map(|value| Some(EntityId(value)))
    } else {
        Ok(None)
    }
}

fn grid(args: &Map<String, Value>) -> Result<Grid, String> {
    Ok(Grid {
        count_x: id(args, "count_x")?,
        spacing_x: optional_number(args, "spacing_x")?.unwrap_or(0.0),
        count_y: if args.contains_key("count_y") {
            id(args, "count_y")?
        } else {
            1
        },
        spacing_y: optional_number(args, "spacing_y")?.unwrap_or(0.0),
    })
}

fn side(args: &Map<String, Value>) -> Result<cndt_model::cam::Side, String> {
    match text(args, "side")?.as_str() {
        "top" => Ok(cndt_model::cam::Side::Top),
        "bottom" => Ok(cndt_model::cam::Side::Bottom),
        "front" => Ok(cndt_model::cam::Side::Front),
        "back" => Ok(cndt_model::cam::Side::Back),
        "left" => Ok(cndt_model::cam::Side::Left),
        "right" => Ok(cndt_model::cam::Side::Right),
        "turned" => Ok(cndt_model::cam::Side::Turned),
        other => Err(format!(
            "unknown side {other:?}; use top, bottom, front, back, left, right or turned"
        )),
    }
}

fn tool_kind(args: &Map<String, Value>) -> Result<cndt_model::cam::ToolKind, String> {
    use cndt_model::cam::ToolKind as K;
    Ok(match text(args, "kind")?.as_str() {
        "spot_drill" => K::SpotDrill,
        "drill" => K::Drill,
        "tap" => K::Tap {
            thread: text(args, "thread")?,
        },
        "reamer" => K::Reamer,
        "end_mill" => K::EndMill,
        "face_mill" => K::FaceMill,
        "chamfer_mill" => K::ChamferMill,
        other => {
            return Err(format!(
                "unknown tool kind {other:?}; use spot_drill, drill, tap, reamer, end_mill, face_mill or chamfer_mill"
            ));
        }
    })
}

fn cam(name: &'static str, edit: cndt_model::cam::CamEdit) -> Result<Box<dyn Command>, String> {
    Ok(Box::new(EditCam::new(name, edit)))
}

/// The `feature` argument of the timeline commands.
/// The arguments of `feature.revolve`; `feature.edit_revolve` takes all but
/// the first, after the feature id.
const REVOLVE_ARGS: &[ArgSpec] = &[
    SKETCH,
    ArgSpec {
        name: "angle",
        kind: ArgKind::Number,
        required: false,
        description: "Degrees, more than 0 up to 360; 360 (a full turn) by default. May be an expression of the parameters.",
    },
    ArgSpec {
        name: "axis",
        kind: ArgKind::Text,
        required: false,
        description: "x or y: the sketch plane's own axis to turn about; y by default. Or give axis_line.",
    },
    ArgSpec {
        name: "axis_line",
        kind: ArgKind::Id,
        required: false,
        description: "A line of the sketch to turn about, usually construction geometry. Instead of axis.",
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
        description: "Which regions; by default every region that is material.",
    },
    ArgSpec {
        name: "name",
        kind: ArgKind::Text,
        required: false,
        description: "Defaults to the next RevolveN.",
    },
    AXIS_WORK,
];

const EDIT_REVOLVE_ARGS: &[ArgSpec] = &[
    FEATURE,
    REVOLVE_ARGS[1],
    REVOLVE_ARGS[2],
    REVOLVE_ARGS[3],
    REVOLVE_ARGS[4],
    REVOLVE_ARGS[5],
    AXIS_WORK,
];

/// The axis a revolve turns about, if the arguments name one.
fn revolve_axis(args: &Map<String, Value>) -> Result<Option<RevolveAxis>, String> {
    let given = ["axis", "axis_line", "axis_work"]
        .iter()
        .filter(|key| args.contains_key(**key))
        .count();
    if args.contains_key("axis") && args.contains_key("axis_line") && given == 2 {
        return Err(String::from("give axis or axis_line, not both"));
    }
    if given > 1 {
        return Err(String::from(
            "give one of axis, axis_line or axis_work, not several",
        ));
    }
    if let Some(reference) = axis_work(args)? {
        return Ok(Some(RevolveAxis::Work { reference }));
    }
    if args.contains_key("axis_line") {
        return Ok(Some(RevolveAxis::Line {
            entity: EntityId(id(args, "axis_line")?),
        }));
    }
    if !args.contains_key("axis") {
        return Ok(None);
    }
    match text(args, "axis")?.as_str() {
        "x" => Ok(Some(RevolveAxis::SketchX)),
        "y" => Ok(Some(RevolveAxis::SketchY)),
        other => Err(format!(
            "unknown axis {other:?}; use x or y, or give axis_line or axis_work"
        )),
    }
}

const WORK_X: ArgSpec = ArgSpec {
    name: "x",
    kind: ArgKind::Number,
    required: false,
    description: "X in model coordinates, mm. 0 by default.",
};
const WORK_Y: ArgSpec = ArgSpec {
    name: "y",
    kind: ArgKind::Number,
    required: false,
    description: "Y in model coordinates, mm. 0 by default.",
};
const WORK_Z: ArgSpec = ArgSpec {
    name: "z",
    kind: ArgKind::Number,
    required: false,
    description: "Z in model coordinates, mm. 0 by default.",
};

const WORK_NAME: ArgSpec = ArgSpec {
    name: "name",
    kind: ArgKind::Text,
    required: false,
    description: "Defaults to the next PlaneN, AxisN or PointN.",
};

fn optional_text(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    if args.contains_key(key) {
        text(args, key)
    } else {
        Ok(String::new())
    }
}

/// The three-number argument pair of a work point or axis: `x`, `y`, `z`
/// with a prefix (`d` for a direction), each 0 when left out.
fn triple(args: &Map<String, Value>, prefix: &str) -> Result<[f64; 3], String> {
    let mut out = [0.0; 3];
    for (slot, axis) in out.iter_mut().zip(["x", "y", "z"]) {
        *slot = optional_number(args, &format!("{prefix}{axis}"))?.unwrap_or(0.0);
    }
    Ok(out)
}

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
            ("cam.add_setup", json!({"side": "upside"}), "unknown side"),
            (
                "cam.add_operation",
                json!({"setup": 0, "type": "engrave"}),
                "unknown operation",
            ),
            (
                "cam.set_tool",
                json!({"number": 1, "kind": "saw", "diameter": 1, "rpm": 1, "feed": 1}),
                "unknown tool kind",
            ),
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
            (
                "feature.revolve",
                json!({"sketch": 0, "angle": 270, "axis": "x", "operation": "cut"}),
            ),
            (
                "feature.edit_revolve",
                json!({"feature": 0, "angle": "sweep / 2", "axis_line": 3, "profiles": [[4, 5, 6, 7]]}),
            ),
            ("feature.delete", json!({"feature": 0})),
            ("feature.suppress", json!({"feature": 0})),
            ("feature.move", json!({"feature": 0, "position": 1})),
            ("cam.set_stock", json!({"side": 2, "top": 2, "bottom": 3})),
            ("cam.add_setup", json!({"side": "top"})),
            ("cam.remove_setup", json!({"setup": 0})),
            (
                "cam.add_operation",
                json!({"setup": 0, "type": "contour", "face": "f0/start/4-5-6-7", "tool": 11, "depth": 20}),
            ),
            ("cam.remove_operation", json!({"setup": 0, "operation": 0})),
            (
                "cam.set_tool",
                json!({"number": 20, "kind": "tap", "thread": "M5x0.8", "diameter": 5, "rpm": 500, "feed": 400}),
            ),
            ("cam.remove_tool", json!({"number": 20})),
            (
                "feature.pattern",
                json!({"feature": 1, "count_x": 4, "spacing_x": 20}),
            ),
            (
                "feature.edit_pattern",
                json!({"feature": 2, "count_x": 3, "spacing_x": 20, "count_y": 2, "spacing_y": 10}),
            ),
            (
                "feature.circular_pattern",
                json!({"feature": 1, "count": 6, "angle": 360, "center": 2}),
            ),
            (
                "feature.edit_circular_pattern",
                json!({"feature": 2, "count": 4, "angle": 90}),
            ),
            ("feature.mirror", json!({"feature": 1, "plane": "sketch_x"})),
            (
                "reference.add_work_plane",
                json!({"plane": "xy", "offset": 5}),
            ),
            (
                "reference.add_work_axis",
                json!({"x": 1, "dz": 1, "name": "A"}),
            ),
            ("reference.add_work_point", json!({"x": 1, "y": 2})),
            ("reference.delete", json!({"reference": 0})),
            (
                "feature.edit_mirror",
                json!({"feature": 2, "plane": "model_z"}),
            ),
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
                "feature.shell",
                json!({"faces": ["f0/end/4-5-6-7"], "thickness": 1.5}),
            ),
            ("feature.edit_shell", json!({"feature": 3, "thickness": 2})),
            (
                "cam.set_vise",
                json!({"setup": 0, "grip": 6, "along": "y", "jaw": 20}),
            ),
            (
                "feature.import",
                json!({"source": "bracket.step", "content": "ISO-10303-21;", "operation": "cut"}),
            ),
            (
                "feature.hole",
                json!({"sketch": 0, "thread": "M6", "depth": 15}),
            ),
            (
                "feature.edit_hole",
                json!({"feature": 0, "diameter": 5.5, "depth": "through", "counterbore_diameter": 9.5, "counterbore_depth": 5.4}),
            ),
            (
                "cam.add_operations",
                json!({"setup": 0, "operations": [{"type": "face"}, {"type": "pocket", "floor": "f1/end/e0", "finish": 0.2}]}),
            ),
            ("sketch.create", json!({"face": "f0/end/4-5-6-7"})),
            (
                "sketch.import",
                json!({"curves": [{"type": "line", "start": [0, 0], "end": [10, 0]}, {"type": "circle", "center": [5, 5], "radius": 2}]}),
            ),
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
                "sketch.add_slot",
                json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 20, "y": 0}, "width": 6}),
            ),
            (
                "sketch.add_spline",
                json!({"sketch": 0, "through": [{"x": 0, "y": 0}, {"x": 5, "y": 4}, {"x": 10, "y": 0}]}),
            ),
            (
                "sketch.trim",
                json!({"sketch": 0, "entity": 1, "at": {"x": 5, "y": 0}}),
            ),
            (
                "sketch.offset",
                json!({"sketch": 0, "entities": [1], "distance": 2, "toward": {"x": 0, "y": 5}}),
            ),
            (
                "sketch.project_edge",
                json!({"sketch": 0, "first": "f0/end/4-5-6-7", "second": "f0/side/e4",
                       "shape": {"type": "line", "start": {"x": 0, "y": 0}, "end": {"x": 10, "y": 0}}}),
            ),
            (
                "sketch.mirror",
                json!({"sketch": 0, "entities": [1], "axis": 4}),
            ),
            (
                "sketch.pattern_linear",
                json!({"sketch": 0, "entities": [1], "count": 3, "step": {"x": 5, "y": 0}}),
            ),
            (
                "sketch.pattern_circular",
                json!({"sketch": 0, "entities": [1], "count": 3, "center": {"x": 0, "y": 0}}),
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
