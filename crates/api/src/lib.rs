// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 5: the one API every front end uses (architecture rule 5).
//!
//! A front end sends a [`Request`] and gets back a [`Response`], both plain
//! JSON. The desktop app carries them over Tauri IPC, the web version over
//! WebAssembly calls, and the CLI over standard input and output. None of
//! them touches the engine directly.
//!
//! Example request and response:
//!
//! ```json
//! {"method": "execute", "command": "parameter.set",
//!  "args": {"name": "thickness", "value": 12, "unit": "mm"}}
//! ```
//!
//! ```json
//! {"ok": true, "result": {"document": {...}, "dirty": true, ...}}
//! ```

mod dxf;
mod dxf_import;
mod export;
mod setup_sheet;
mod simulate;
mod thumbnail;
mod verify;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cndt_engine::{CommandError, Registry, Session};
use cndt_format::{FormatError, Project};
use cndt_geom::TriangleMesh;
use cndt_kernel::{Kernel, KernelError, OpenCascade};
use cndt_model::sketch::Freedom;
use cndt_model::{
    Document, FeatureId, FeatureStatus, FrameStatus, ModelError, Rebuilt, SketchError, SketchId,
    rebuild_to,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Everything a front end can ask for.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    /// Start a new, empty document.
    NewDocument {
        /// The document's name.
        name: String,
        /// Throw away unsaved changes in the current document.
        #[serde(default)]
        discard_changes: bool,
    },
    /// Open a project file.
    Open {
        /// The file to open.
        path: PathBuf,
        /// Throw away unsaved changes in the current document.
        #[serde(default)]
        discard_changes: bool,
    },
    /// Save to the file the document was opened from or last saved to.
    Save,
    /// Save to a new file, which becomes the document's file.
    SaveAs {
        /// The file to write.
        path: PathBuf,
    },
    /// Run a registered command.
    Execute {
        /// The command's registered name.
        command: String,
        /// Its arguments, as a JSON object.
        #[serde(default)]
        args: Value,
    },
    /// Undo the last command.
    Undo,
    /// Redo the last undone command.
    Redo,
    /// Return the current state.
    State,
    /// Describe every registered command and its arguments.
    Commands,
    /// The thumbnail stored in a project file (decision 0025), without
    /// opening it. Replies `{png_base64}`, null when the file has none.
    ProjectThumbnail {
        /// The project file.
        path: PathBuf,
    },
    /// Summarise a STEP file's topology and size without importing it.
    StepInfo {
        /// The STEP file.
        path: PathBuf,
    },
    /// G-code for one stored setup (decision 0014), through a shipped
    /// post. Replies `{gcode, notes}`; with `path`, also writes the G-code
    /// there.
    Post {
        /// `fanuc-mill`, `haas-mill`, `linuxcnc-mill`, `mazak-smoothg` or
        /// `grbl`. Not needed with `definition`.
        #[serde(default)]
        post: String,
        /// A post definition as TOML text, instead of a shipped post: one
        /// edited in the post editor. If it is a shipped post changed, or
        /// not shipped at all, the program header says so.
        #[serde(default)]
        definition: Option<String>,
        /// A shipped machine, such as `mazak-hcn-5000`, to check the program
        /// against first (decision 0015). A program that breaks one of its
        /// limits produces no G-code.
        #[serde(default)]
        machine: Option<String>,
        /// The setup's id; the first setup if left out.
        #[serde(default)]
        setup: Option<u32>,
        /// Several setups in one program, in this order, instead of
        /// `setup`: the table turns between sides that have a B angle, and
        /// the program stops for the operator to turn the part between the
        /// others.
        #[serde(default)]
        setups: Option<Vec<u32>>,
        /// Post even though verification found problems (a rapid into the
        /// stock, say). The program's header then lists every problem
        /// overridden. Without it, such a program is refused.
        #[serde(default, rename = "override")]
        override_problems: bool,
        /// A prove-out: every Z raised by this many millimetres, so the
        /// machine runs the whole program in the air above the part. The
        /// real program is verified as usual; the header says it cuts air.
        #[serde(default)]
        prove_out: Option<f64>,
        /// A file to write the program to.
        #[serde(default)]
        path: Option<PathBuf>,
    },
    /// Stock simulation of one setup (decision 0016): its program swept
    /// through the raw stock and checked against the exact part. Changes
    /// nothing; replies `{collisions, gouges, left, left_at, removed, cut_length,
    /// rapid_length, cut_minutes, grid}`. A collision is a rapid move into
    /// material; a gouge, a cut below the finished part.
    Simulate {
        /// The setup's id; the first setup if left out.
        #[serde(default)]
        setup: Option<u32>,
        /// The cell size in millimetres; 0.25 if left out.
        #[serde(default)]
        cell: Option<f64>,
        /// The setups machined before it, in order, whose stock it starts
        /// from; the setups before it in the document if left out, a fresh
        /// block if empty.
        #[serde(default)]
        after: Option<Vec<u32>>,
    },
    /// The verification report (decision 0016): the setups (all, in
    /// order, if left out) as one program through `post`, on `machine` if
    /// given, checked every way CNDT can: plans, machine limits, posting,
    /// and a stock simulation of each setup. Changes nothing; replies
    /// `{verdict, problems, review, text}`, the verdict being `problems`
    /// (do not run), `review` or `clear`. With `path`, also writes `text`
    /// there.
    Verify {
        /// A shipped post, as for `post`.
        #[serde(default)]
        post: String,
        /// A post definition as TOML text, as for `post`.
        #[serde(default)]
        definition: Option<String>,
        /// A shipped machine, as for `post`.
        #[serde(default)]
        machine: Option<String>,
        /// The setups, in machining order.
        #[serde(default)]
        setups: Option<Vec<u32>>,
        /// The simulation's cell size in millimetres; 0.5 if left out.
        #[serde(default)]
        cell: Option<f64>,
        /// A file to write the report to.
        #[serde(default)]
        path: Option<PathBuf>,
    },
    /// Writes the setup sheet: a printable HTML page for the operator at
    /// the machine, with each setup's work offset, zero, stock, tools,
    /// cutting time and every verification finding. Changes nothing;
    /// replies `{html, setups}`. With `path`, also writes the page there.
    /// The post defaults to the machine's own, else `fanuc-mill`.
    SetupSheet {
        /// A shipped post.
        #[serde(default)]
        post: Option<String>,
        /// A post definition as TOML text, as for `post`.
        #[serde(default)]
        definition: Option<String>,
        /// A shipped machine, as for `post`.
        #[serde(default)]
        machine: Option<String>,
        /// The setups, in machining order; all of them if left out.
        #[serde(default)]
        setups: Option<Vec<u32>>,
        /// A file to write the page to.
        #[serde(default)]
        path: Option<PathBuf>,
    },
    /// Writes the part, as built at the history marker, to a file: STEP
    /// (`.step`, `.stp`) or binary STL (`.stl`) by the extension. Replies
    /// `{format}`, and for STL `triangles`.
    Export {
        /// The file to write.
        path: PathBuf,
    },
    /// Writes a DXF (R12, millimetres) of a sketch, exactly and in its own
    /// coordinates, or of the part cut through at a height, seen from
    /// above. Replies `{entities}`.
    ExportDxf {
        /// The file to write.
        path: PathBuf,
        /// A sketch's id.
        #[serde(default)]
        sketch: Option<u64>,
        /// A model height to cut the part through at, instead of a sketch:
        /// its true walls there, outside and holes.
        #[serde(default)]
        section: Option<f64>,
    },
    /// Reads a DXF drawing's lines, arcs, circles and polylines into a new
    /// sketch, as one step that undo takes back. Ends that meet become one
    /// point, so closed outlines extrude. Replies with the state and
    /// `import_notes`: the units assumed or converted, and anything left out.
    ImportDxf {
        /// The DXF file.
        path: PathBuf,
        /// xy (the default), xz or yz.
        #[serde(default)]
        plane: Option<String>,
        /// The sketch's name; the file's name if left out.
        #[serde(default)]
        name: Option<String>,
    },
    /// Adds the solid in a STEP or IGES file to the timeline, joined to the
    /// part or cut from it (decision 0017); which format is told from the
    /// content. The file's content is kept in the
    /// project. Replies with the state.
    ImportStep {
        /// The STEP or IGES file.
        path: PathBuf,
        /// join (the default) or cut.
        #[serde(default)]
        operation: Option<String>,
        /// The feature's name; the next ImportN if left out.
        #[serde(default)]
        name: Option<String>,
    },
    /// Adds an STL file's triangles to the timeline as a reference mesh
    /// (decision 0018): shown and measured beside the part, never joined to
    /// it or machined. The file's content is kept in the project. Replies
    /// with the state.
    ImportMesh {
        /// The STL file, binary or ASCII, in millimetres.
        path: PathBuf,
        /// The feature's name; the next MeshN if left out.
        #[serde(default)]
        name: Option<String>,
    },
    /// A shipped post's TOML text, to start editing from. Replies
    /// `{text}`.
    PostSource {
        /// The shipped post's id.
        post: String,
    },
    /// The post editor's preview: a setup (or several) posted through a
    /// definition being edited, without verifying. Replies `{gcode,
    /// changed, base}`: `changed` lists the lines (from 0) that differ from
    /// what the shipped post of the same id gives, `base` names that post
    /// (`null` for a post not shipped).
    PostPreview {
        /// The definition as TOML text.
        definition: String,
        /// The setup's id; the first setup if left out.
        #[serde(default)]
        setup: Option<u32>,
        /// Several setups in one program, instead of `setup`.
        #[serde(default)]
        setups: Option<Vec<u32>>,
    },
    /// Reads a post definition file. Replies `{text}`; a file that is not a
    /// valid post is refused with the reason.
    ReadPost {
        /// The `.toml` file.
        path: PathBuf,
    },
    /// Writes a post definition to a file, after checking it is a valid
    /// post. Replies `{}`.
    WritePost {
        /// The definition as TOML text.
        definition: String,
        /// The file to write.
        path: PathBuf,
    },
    /// Moves the history marker: the part is rebuilt from the first `to`
    /// features only, as it was at that point, or from all of them when
    /// `to` is left out. A view, not a change: the document is untouched,
    /// and any change to it brings the marker back to the end. Replies with
    /// the state, whose `rollback` says where the marker is.
    Rollback {
        /// How many features to build; all of them if left out.
        #[serde(default)]
        to: Option<usize>,
    },
    /// Feature detection: the operations suggested for a setup from the
    /// part's holes, pockets and outside profile. Changes nothing; replies
    /// `{operations, notes}`, and `operations` can be passed straight to
    /// the `cam.add_operations` command.
    Detect {
        /// The setup's id.
        setup: u32,
    },
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    /// Whether the request succeeded.
    pub ok: bool,
    /// The result, on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// What went wrong, on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

/// A failure, with a stable machine-readable kind and a message for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiError {
    /// A stable identifier front ends can branch on, such as `no_path`.
    pub kind: &'static str,
    /// A readable explanation.
    pub message: String,
    /// Structured detail for some kinds. `conflict` and `redundant` carry
    /// `{"sketch": id, "constraints": [ids]}` so the interface can highlight
    /// them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            details: None,
        }
    }
}

impl From<CommandError> for ApiError {
    fn from(error: CommandError) -> Self {
        if let CommandError::Model(ModelError::Sketch(
            SketchError::Conflict { constraints } | SketchError::Redundant { constraints },
        )) = &error
        {
            let kind = if matches!(
                error,
                CommandError::Model(ModelError::Sketch(SketchError::Conflict { .. }))
            ) {
                "conflict"
            } else {
                "redundant"
            };
            let ids: Vec<u32> = constraints.iter().map(|id| id.0).collect();
            let mut api = Self::new(kind, error.to_string());
            api.details = Some(serde_json::json!({ "constraints": ids }));
            return api;
        }
        let kind = match &error {
            CommandError::UnknownCommand(_) => "unknown_command",
            CommandError::InvalidArguments { .. } => "invalid_arguments",
            CommandError::Model(_) => "rejected",
            CommandError::NotApplied => "internal",
            CommandError::NothingToUndo => "nothing_to_undo",
            CommandError::NothingToRedo => "nothing_to_redo",
        };
        Self::new(kind, error.to_string())
    }
}

impl From<KernelError> for ApiError {
    fn from(error: KernelError) -> Self {
        let kind = match &error {
            KernelError::Unreadable { .. } => "unreadable",
            KernelError::Empty { .. } => "empty",
            KernelError::Internal(_) => "kernel",
            KernelError::Modelling(_) => "modelling",
        };
        Self::new(kind, error.to_string())
    }
}

impl From<FormatError> for ApiError {
    fn from(error: FormatError) -> Self {
        let kind = match &error {
            FormatError::Io(_) => "io",
            FormatError::NotAProject(_) => "not_a_project",
            FormatError::NewerVersion { .. } => "newer_version",
            FormatError::Corrupt(_) | FormatError::BadEntryName(_) => "corrupt",
            FormatError::InvalidContent(_) => "invalid_content",
        };
        Self::new(kind, error.to_string())
    }
}

/// The state a front end displays after every change.
#[derive(Debug, Serialize)]
struct StateView<'a> {
    document: &'a Document,
    path: Option<String>,
    dirty: bool,
    undo: Option<String>,
    redo: Option<String>,
    /// Degrees of freedom of each sketch, for colouring what is fully
    /// constrained.
    freedom: BTreeMap<SketchId, Freedom>,
    /// How each timeline feature built.
    feature_status: &'a BTreeMap<FeatureId, FeatureStatus>,
    /// Where each sketch sits in the model, for drawing it in 3D.
    sketch_frames: &'a BTreeMap<SketchId, FrameStatus>,
    /// The rebuilt part, if any feature made one.
    body: Option<BodyView>,
    /// Goes up by one on every rebuild; fetch `body_mesh` again when it
    /// changes.
    body_revision: u64,
    /// How many timeline features are built, when the history is rolled
    /// back; `null` for all of them.
    rollback: Option<usize>,
    /// Each library tool's assembly as the checks use it, and whether it
    /// was measured or is typical for the tool.
    tool_assemblies: BTreeMap<u32, ToolAssemblyView>,
    /// Each reference mesh that read (decision 0018): its size, for
    /// measuring. Its triangles come from `reference_mesh`.
    references: BTreeMap<FeatureId, ReferenceView>,
}

/// A reference mesh in the state.
#[derive(Debug, Serialize)]
struct ReferenceView {
    triangles: usize,
    bounds: cndt_geom::BoundingBox,
    /// Millimetres along X, Y and Z.
    size: [f64; 3],
}

/// A tool's assembly in the state.
#[derive(Debug, Serialize)]
struct ToolAssemblyView {
    #[serde(flatten)]
    assembly: cndt_model::cam::Assembly,
    measured: bool,
}

/// The part as front ends see it. Its triangles come from `body_mesh`.
#[derive(Debug, Serialize)]
struct BodyView {
    /// Cubic millimetres.
    volume: f64,
    /// Bounding box.
    bounds: cndt_geom::BoundingBox,
    /// The persistent names of each face, by the face numbers `body_mesh`
    /// uses (decision 0012).
    faces: Vec<Vec<String>>,
    /// The two faces each edge joins, by the edge numbers `body_edges`
    /// uses. An edge is referred to by its two faces' names.
    edges: Vec<(u32, u32)>,
}

/// One open document and everything needed to work on it.
#[derive(Debug)]
pub struct Api {
    session: Session,
    registry: Registry,
    path: Option<PathBuf>,
    /// Module data from the opened file, written back unchanged on save.
    extensions: BTreeMap<String, Vec<u8>>,
    kernel: Box<dyn Kernel>,
    rebuilt: Rebuilt,
    revision: u64,
    /// The last stock simulation, for drawing it.
    simulated: Option<simulate::Simulated>,
    /// The last verification and what it was of, so posting right after
    /// verifying does not simulate again.
    verified: Option<(String, verify::Verification)>,
    /// The history marker: how many features are built, or all of them.
    /// A view of the past, not part of the document; any change to the
    /// document brings it back to the end.
    rollback: Option<usize>,
}

impl Default for Api {
    fn default() -> Self {
        Self::new()
    }
}

/// Identifies the binary mesh layout returned by [`Api::step_preview`].
pub const MESH_MAGIC: &[u8; 8] = b"CNDTMSH1";

/// The posts that ship with CNDT (`posts/` in the repository).
const SHIPPED_POSTS: &[(&str, &str)] = &[
    ("fanuc-mill", include_str!("../../../posts/fanuc-mill.toml")),
    ("grbl", include_str!("../../../posts/grbl.toml")),
    ("haas-mill", include_str!("../../../posts/haas-mill.toml")),
    (
        "linuxcnc-mill",
        include_str!("../../../posts/linuxcnc-mill.toml"),
    ),
    (
        "mazak-smoothg",
        include_str!("../../../posts/mazak-smoothg.toml"),
    ),
];

/// The machine definitions that ship with CNDT (`machines/`, decision 0015).
const SHIPPED_MACHINES: &[(&str, &str)] = &[(
    "mazak-hcn-5000",
    include_str!("../../../machines/mazak-hcn-5000.toml"),
)];

/// Reads a shipped machine definition by id.
fn shipped_machine(id: &str) -> Result<cndt_cam_core::Machine, ApiError> {
    let text = SHIPPED_MACHINES
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, text)| *text)
        .ok_or_else(|| {
            let names: Vec<&str> = SHIPPED_MACHINES.iter().map(|(name, _)| *name).collect();
            ApiError::new(
                "unknown_machine",
                format!(
                    "there is no machine {id:?}; use one of {}",
                    names.join(", ")
                ),
            )
        })?;
    cndt_cam_core::Machine::from_toml(text).map_err(|e| ApiError::new("internal", e.to_string()))
}

/// Which post to use: a shipped one by id, or a definition given as TOML
/// text, such as one edited in the post editor.
#[derive(Debug, Clone)]
pub(crate) struct PostRef {
    id: String,
    definition: Option<String>,
}

impl PostRef {
    fn shipped_text(id: &str) -> Option<&'static str> {
        SHIPPED_POSTS
            .iter()
            .find(|(name, _)| *name == id)
            .map(|(_, text)| *text)
    }

    /// The loaded post, its id, and a header line saying it is not a
    /// shipped post as shipped, if so.
    fn load(&self) -> Result<(cndt_post::Post, String, Option<String>), ApiError> {
        match &self.definition {
            None => {
                let text = Self::shipped_text(&self.id).ok_or_else(|| {
                    let names: Vec<&str> = SHIPPED_POSTS.iter().map(|(name, _)| *name).collect();
                    ApiError::new(
                        "unknown_post",
                        format!(
                            "there is no post {:?}; use one of {}",
                            self.id,
                            names.join(", ")
                        ),
                    )
                })?;
                let post = cndt_post::Post::from_toml(text)
                    .map_err(|e| ApiError::new("internal", e.to_string()))?;
                Ok((post, self.id.clone(), None))
            }
            Some(text) => {
                let post = cndt_post::Post::from_toml(text)
                    .map_err(|e| ApiError::new("post_definition", e.to_string()))?;
                let id = post.definition().post.id.clone();
                let note = match Self::shipped_text(&id) {
                    Some(shipped) if shipped == text => None,
                    Some(_) => Some(format!("EDITED POST: CHANGED FROM THE SHIPPED {id}")),
                    None => Some(format!("CUSTOM POST {id}: NOT SHIPPED WITH CNDT")),
                };
                Ok((post, id, note))
            }
        }
    }

    /// Identifies the post for caching: its id and, for a definition, a
    /// hash of its text.
    fn key(&self) -> String {
        match &self.definition {
            None => self.id.clone(),
            Some(text) => {
                // FNV-1a: stable on every platform.
                let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
                });
                format!("{}#{hash:016x}", self.id)
            }
        }
    }
}

impl std::fmt::Display for PostRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.load() {
            Ok((_, id, Some(_))) => write!(f, "{id} (edited)"),
            Ok((_, id, None)) => f.write_str(&id),
            Err(_) => f.write_str(&self.id),
        }
    }
}

/// Line numbers (from 0) of `new` that are not in a longest common
/// subsequence with `old`: what an edit changed. `None` if too long to
/// compare.
fn changed_lines(old: &str, new: &str) -> Option<Vec<usize>> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a.len().saturating_mul(b.len()) > 25_000_000 {
        return None;
    }
    // lcs[i][j]: the longest common subsequence of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut lcs = vec![0_u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut changed = Vec::new();
    while j < b.len() {
        if i < a.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < a.len() && lcs[(i + 1) * width + j] >= lcs[i * width + j + 1] {
            i += 1;
        } else {
            changed.push(j);
            j += 1;
        }
    }
    Some(changed)
}

/// The first bytes of a toolpath preview buffer.
pub const TOOLPATH_MAGIC: &[u8; 8] = b"CNDTTPH2";

/// The first bytes of a `body_edges` buffer. Layout, little-endian: magic;
/// u32 edge count `n`; `n` u32 point counts; then every edge's points as
/// f32 x, y, z, edge after edge, in the order of `body.edges`.
pub const EDGES_MAGIC: &[u8; 8] = b"CNDTEDG1";

impl Api {
    /// Starts with an empty, unsaved document and the OpenCascade kernel.
    #[must_use]
    pub fn new() -> Self {
        Self::with_kernel(Box::new(OpenCascade))
    }

    /// Starts with an empty, unsaved document and the given kernel.
    #[must_use]
    pub fn with_kernel(kernel: Box<dyn Kernel>) -> Self {
        Self {
            session: Session::new(Document::default()),
            registry: Registry::standard(),
            path: None,
            extensions: BTreeMap::new(),
            kernel,
            rebuilt: Rebuilt::default(),
            revision: 0,
            simulated: None,
            rollback: None,
            verified: None,
        }
    }

    /// Rebuilds the part from the timeline after the document changed.
    fn refresh(&mut self) {
        self.rebuilt = rebuild_to(self.session.document(), self.kernel.as_ref(), self.rollback);
        self.revision += 1;
    }

    /// The rebuilt part's edges as polylines, in the layout documented on
    /// [`EDGES_MAGIC`]. Edge numbers match `body.edges` in the state.
    ///
    /// # Errors
    ///
    /// `no_body` when nothing has been built yet; a kernel error.
    pub fn body_edges(&self) -> Result<Vec<u8>, ApiError> {
        let body = self
            .rebuilt
            .body
            .as_ref()
            .ok_or_else(|| ApiError::new("no_body", "there is no solid yet"))?;
        let deflection = self.deflection();
        let lines = self.kernel.edge_polylines(body, deflection)?;
        let too_big = || ApiError::new("internal", "too many edges to send");
        let mut out = Vec::new();
        out.extend_from_slice(EDGES_MAGIC);
        out.extend_from_slice(
            &u32::try_from(lines.len())
                .map_err(|_| too_big())?
                .to_le_bytes(),
        );
        for line in &lines {
            out.extend_from_slice(
                &u32::try_from(line.len() / 3)
                    .map_err(|_| too_big())?
                    .to_le_bytes(),
            );
        }
        for line in &lines {
            for value in line {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        Ok(out)
    }

    /// Plans the hole program for the current part and posts it.
    /// Posts after verifying (docs/safety.md, gate 4): a program with
    /// verification problems is refused unless the user overrides them, and
    /// then the header lists what was overridden.
    fn post(
        &mut self,
        id: &PostRef,
        machine: Option<&str>,
        setup: Option<u32>,
        setups: Option<&[u32]>,
        output: Output<'_>,
        override_problems: bool,
    ) -> Result<Value, ApiError> {
        // Its own errors first, with their own kinds.
        let check = Output {
            path: None,
            ..output
        };
        self.post_program(id, machine, setup, setups, check, &[])?;
        let ids: Vec<u32> = match (setup, setups) {
            (_, Some(ids)) => ids.to_vec(),
            (Some(setup), None) => vec![setup],
            (None, None) => self
                .session
                .document()
                .cam()
                .setups
                .first()
                .map(|s| s.id)
                .into_iter()
                .collect(),
        };
        let problems = self.verification(id, machine, &ids)?.problems;
        let mut notes = Vec::new();
        if !problems.is_empty() {
            if !override_problems {
                return Err(ApiError::new(
                    "verification",
                    format!(
                        "verification found {} problem{}; resolve {} or post with override: {}",
                        problems.len(),
                        if problems.len() == 1 { "" } else { "s" },
                        if problems.len() == 1 { "it" } else { "them" },
                        problems.join(" ")
                    ),
                ));
            }
            notes.push(String::from("VERIFICATION OVERRIDDEN BY THE USER:"));
            notes.extend(problems);
        }
        self.post_program(id, machine, setup, setups, output, &notes)
    }

    /// Imports a DXF drawing as a new sketch (see [`Request::ImportDxf`]).
    fn import_dxf(
        &mut self,
        path: &Path,
        plane: Option<String>,
        name: Option<String>,
    ) -> Result<Value, ApiError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ApiError::new("io", format!("cannot read {}: {e}", path.display())))?;
        let drawing = dxf_import::read(&text).map_err(|e| ApiError::new("import", e))?;
        let name = name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned()
        });
        let mut args = serde_json::Map::new();
        args.insert(
            String::from("curves"),
            serde_json::to_value(&drawing.curves)
                .map_err(|e| ApiError::new("internal", e.to_string()))?,
        );
        args.insert(String::from("name"), Value::from(name));
        args.insert(
            String::from("plane"),
            Value::from(plane.unwrap_or_else(|| String::from("xy"))),
        );
        let command = self
            .registry
            .build("sketch.import", &Value::Object(args))
            .map_err(ApiError::from)?;
        self.session.execute(command).map_err(ApiError::from)?;
        self.rollback = None;
        self.refresh();
        let mut state = self.state()?;
        state["import_notes"] = serde_json::json!(drawing.notes);
        Ok(state)
    }

    /// Imports a STEP file's solid (see [`Request::ImportStep`]).
    fn import_step(
        &mut self,
        path: &Path,
        operation: Option<String>,
        name: Option<String>,
    ) -> Result<Value, ApiError> {
        let size = std::fs::metadata(path)
            .map_err(|e| ApiError::new("io", format!("cannot read {}: {e}", path.display())))?
            .len();
        if size > cndt_model::MAX_IMPORT_BYTES {
            return Err(ApiError::new(
                "import",
                format!(
                    "{} is {size} bytes; the most an import may be is {}",
                    path.display(),
                    cndt_model::MAX_IMPORT_BYTES
                ),
            ));
        }
        let content = std::fs::read_to_string(path).map_err(|e| {
            ApiError::new(
                "import",
                format!("cannot read {} as STEP or IGES text: {e}", path.display()),
            )
        })?;
        let source = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("import.step")
            .to_owned();
        let mut args = serde_json::Map::new();
        args.insert(String::from("source"), Value::from(source));
        args.insert(String::from("content"), Value::from(content));
        if let Some(operation) = operation {
            args.insert(String::from("operation"), Value::from(operation));
        }
        if let Some(name) = name {
            args.insert(String::from("name"), Value::from(name));
        }
        let command = self
            .registry
            .build("feature.import", &Value::Object(args))
            .map_err(ApiError::from)?;
        self.session.execute(command).map_err(ApiError::from)?;
        self.rollback = None;
        self.refresh();
        self.state()
    }

    /// Adds an STL reference mesh (see [`Request::ImportMesh`]).
    fn import_mesh(&mut self, path: &Path, name: Option<String>) -> Result<Value, ApiError> {
        let io =
            |e: std::io::Error| ApiError::new("io", format!("cannot read {}: {e}", path.display()));
        let size = std::fs::metadata(path).map_err(io)?.len();
        if size > cndt_model::MAX_IMPORT_BYTES {
            return Err(ApiError::new(
                "import",
                format!(
                    "{} is {size} bytes; the most an import may be is {}",
                    path.display(),
                    cndt_model::MAX_IMPORT_BYTES
                ),
            ));
        }
        let content = std::fs::read(path).map_err(io)?;
        let source = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("mesh.stl")
            .to_owned();
        self.session
            .execute(Box::new(cndt_engine::AddReference::new(
                source, content, name,
            )))
            .map_err(ApiError::from)?;
        self.rollback = None;
        self.refresh();
        self.state()
    }

    /// The post editor's preview (see [`Request::PostPreview`]).
    fn post_preview(
        &self,
        definition: String,
        setup: Option<u32>,
        setups: Option<&[u32]>,
    ) -> Result<Value, ApiError> {
        let edited = PostRef {
            id: String::new(),
            definition: Some(definition),
        };
        let (_, id, _) = edited.load()?;
        let ours = self.post_program(&edited, None, setup, setups, Output::default(), &[])?;
        let gcode = ours["gcode"].as_str().unwrap_or_default().to_owned();
        let base = PostRef {
            id: id.clone(),
            definition: None,
        };
        let (changed, base_id) =
            match self.post_program(&base, None, setup, setups, Output::default(), &[]) {
                Ok(theirs) => (
                    changed_lines(theirs["gcode"].as_str().unwrap_or_default(), &gcode),
                    Some(id),
                ),
                // Not a shipped post: every line is its own.
                Err(_) => (Some((0..gcode.lines().count()).collect()), None),
            };
        Ok(serde_json::json!({ "gcode": gcode, "changed": changed, "base": base_id }))
    }

    /// The verification of these setups through this post and machine,
    /// reusing the last one if nothing has changed since.
    fn verification(
        &mut self,
        post: &PostRef,
        machine: Option<&str>,
        ids: &[u32],
    ) -> Result<verify::Verification, ApiError> {
        let key = format!("{}|{}|{machine:?}|{ids:?}", self.revision, post.key());
        if let Some((kept, report)) = &self.verified
            && *kept == key
        {
            return Ok(report.clone());
        }
        let report = self.verify(post, machine, Some(ids), None, None)?;
        self.verified = Some((key, report.clone()));
        Ok(report)
    }

    /// Plans and posts without verifying, adding `header` to the program's
    /// header comment.
    pub(crate) fn post_program(
        &self,
        post_ref: &PostRef,
        machine: Option<&str>,
        setup: Option<u32>,
        setups: Option<&[u32]>,
        output: Output<'_>,
        header: &[String],
    ) -> Result<Value, ApiError> {
        let Output { path, prove_out } = output;
        let (post, id, edited) = post_ref.load()?;
        let id = id.as_str();
        let lift_note = prove_out.map(|lift| {
            format!(
                "PROVE-OUT: EVERY Z IS RAISED {lift:.3} MM. THIS PROGRAM CUTS AIR, NOT THE PART. \
                 DO NOT USE IT TO CUT."
            )
        });
        let header: Vec<String> = lift_note
            .into_iter()
            .chain(edited)
            .chain(header.iter().cloned())
            .collect();
        let header = header.as_slice();
        let planned = match (setup, setups) {
            (Some(_), Some(_)) => {
                return Err(ApiError::new(
                    "bad_request",
                    "give either setup or setups, not both",
                ));
            }
            (_, Some(ids)) => cndt_process_mill::plan_setups(
                self.session.document(),
                &self.rebuilt,
                self.kernel.as_ref(),
                ids,
            ),
            (setup, None) => cndt_process_mill::plan_setup(
                self.session.document(),
                &self.rebuilt,
                self.kernel.as_ref(),
                setup,
            ),
        }
        .map_err(|e| ApiError::new("cam", e.to_string()))?;
        let mut notes = planned.notes;
        let lifted = prove_out
            .map(|lift| planned.program.lifted(lift))
            .transpose()
            .map_err(|e| ApiError::new("bad_request", e))?;
        // A lift smaller than the program's depth would still reach the
        // stock: refuse it, saying how far is enough.
        if let (Some(lift), Some(lifted)) = (prove_out, &lifted) {
            let above = self
                .session
                .document()
                .cam()
                .stock
                .map_or(0.0, |stock| stock.top.max(stock.bottom));
            let clear = above + PROVE_OUT_CLEARANCE;
            if let Some(lowest) = lifted.lowest_z()
                && lowest < clear
            {
                return Err(ApiError::new(
                    "bad_request",
                    format!(
                        "a prove-out of {lift:.3} mm still reaches Z{lowest:.3}, within \
                         {PROVE_OUT_CLEARANCE} mm of the stock; lift at least {:.3} mm",
                        lift + clear - lowest
                    ),
                ));
            }
        }
        if let Some(machine) = machine {
            let machine = shipped_machine(machine)?;
            // Another controller's codes can mean something else here: on a
            // Mazak, the Fanuc post's M29 is a robot request.
            if machine.machine.post != id {
                return Err(ApiError::new(
                    "machine",
                    format!(
                        "the {} takes the {} post, not {id}",
                        machine.machine.name, machine.machine.post
                    ),
                ));
            }
            machine
                .check(&planned.program)
                .map_err(|e| ApiError::new("machine", e.to_string()))?;
            // The lifted program must fit the machine's travel too.
            if let Some(lifted) = &lifted {
                machine.check(lifted).map_err(|e| {
                    ApiError::new("machine", format!("the prove-out does not fit: {e}"))
                })?;
            }
            if !machine.machine.verified {
                notes.push(format!(
                    "The {} definition is unverified: check its limits on the machine.",
                    machine.machine.name
                ));
            }
        }
        let gcode = post
            .run_noted(
                lifted.as_ref().unwrap_or(&planned.program),
                env!("CARGO_PKG_VERSION"),
                header,
            )
            .map_err(|e| ApiError::new("post", e.to_string()))?;
        if let Some(path) = path {
            std::fs::write(path, &gcode).map_err(|e| {
                ApiError::new("io", format!("cannot write {}: {e}", path.display()))
            })?;
        }
        Ok(serde_json::json!({ "gcode": gcode, "notes": notes }))
    }

    /// A setup's toolpath for drawing, in model coordinates. Layout,
    /// little-endian: [`TOOLPATH_MAGIC`]; u32 segment count `n`; u32
    /// operation count `m`; per operation u32 index of its first segment,
    /// u32 name length in bytes and the name as UTF-8; then `n` segments,
    /// each u8 kind (0 rapid, 1 cut) and f32 x, y, z from, x, y, z to
    /// (25 bytes). Operations are in program order, so a segment belongs to
    /// the last operation that starts at or before it.
    ///
    /// # Errors
    ///
    /// `cam` when the setup cannot be planned.
    pub fn toolpath_preview(&self, setup: Option<u32>) -> Result<Vec<u8>, ApiError> {
        let planned = cndt_process_mill::plan_setup(
            self.session.document(),
            &self.rebuilt,
            self.kernel.as_ref(),
            setup,
        )
        .map_err(|e| ApiError::new("cam", e.to_string()))?;
        let Some(placed) = planned.setup else {
            return Err(ApiError::new("cam", "the setup has no placement"));
        };
        let (segments, starts) = cndt_process_mill::preview_operations(&planned.program, &placed);
        let too_long = || ApiError::new("internal", "toolpath too long");
        let mut out = Vec::with_capacity(16 + segments.len() * 25);
        out.extend_from_slice(TOOLPATH_MAGIC);
        out.extend_from_slice(
            &u32::try_from(segments.len())
                .map_err(|_| too_long())?
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(starts.len())
                .map_err(|_| too_long())?
                .to_le_bytes(),
        );
        for (name, first) in &starts {
            out.extend_from_slice(&u32::try_from(*first).map_err(|_| too_long())?.to_le_bytes());
            out.extend_from_slice(
                &u32::try_from(name.len())
                    .map_err(|_| too_long())?
                    .to_le_bytes(),
            );
            out.extend_from_slice(name.as_bytes());
        }
        for s in segments {
            out.push(u8::from(!s.rapid));
            for v in [s.from.x, s.from.y, s.from.z, s.to.x, s.to.y, s.to.z] {
                #[allow(clippy::cast_possible_truncation)]
                out.extend_from_slice(&(v as f32).to_le_bytes());
            }
        }
        Ok(out)
    }

    fn deflection(&self) -> f64 {
        self.rebuilt.properties.map_or(0.05, |p| {
            let [x, y, z] = p.bounds.size();
            (x.hypot(y).hypot(z) * 0.0005).clamp(0.005, 1.0)
        })
    }

    /// The rebuilt part's display mesh, in the layout documented on
    /// [`Api::step_preview`]. Face numbers match `body.faces` in the state.
    ///
    /// # Errors
    ///
    /// `no_body` when nothing has been built yet; a kernel error if
    /// triangulation fails.
    pub fn body_mesh(&self) -> Result<Vec<u8>, ApiError> {
        let body = self
            .rebuilt
            .body
            .as_ref()
            .ok_or_else(|| ApiError::new("no_body", "there is no solid yet"))?;
        let mesh = self.kernel.mesh(body, self.deflection())?;
        encode_mesh(&mesh)
    }

    /// A reference mesh's triangles (decision 0018), in the layout
    /// documented on [`Api::step_preview`], all one face.
    ///
    /// # Errors
    ///
    /// `unknown_feature` when `feature` is not a reference mesh that read.
    pub fn reference_mesh(&self, feature: u32) -> Result<Vec<u8>, ApiError> {
        let reference = self
            .rebuilt
            .references
            .get(&FeatureId(feature))
            .ok_or_else(|| {
                ApiError::new(
                    "unknown_feature",
                    format!("feature {feature} is not a reference mesh that read"),
                )
            })?;
        encode_mesh(&reference.mesh)
    }

    /// The geometry kernel's name and version, such as `OpenCascade 7.9.3`.
    #[must_use]
    pub fn kernel_description(&self) -> String {
        self.kernel.description()
    }

    /// Triangulates a STEP file for display and returns the mesh in binary
    /// (architecture rule 12: geometry never travels as JSON).
    ///
    /// Layout, all little-endian, every array 4-byte aligned:
    ///
    /// | Offset | Content |
    /// | --- | --- |
    /// | 0 | `CNDTMSH1` |
    /// | 8 | `u32` vertex count `v` |
    /// | 12 | `u32` triangle count `t` |
    /// | 16 | `u32` model face count |
    /// | 20 | `f32` positions, `3 v` values, millimetres |
    /// | then | `u32` vertex indices, `3 t` values |
    /// | then | `u32` model face of each triangle, `t` values |
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or triangulated.
    pub fn step_preview(&self, path: &Path) -> Result<Vec<u8>, ApiError> {
        let summary = self.kernel.import_step(path)?;
        // Tolerance follows the part's size: 0.05 % of its diagonal, kept
        // between 0.005 mm and 1 mm, so small and large parts both look right.
        let deflection = summary.bounds.map_or(0.05, |bounds| {
            let [x, y, z] = bounds.size();
            (x.hypot(y).hypot(z) * 0.0005).clamp(0.005, 1.0)
        });
        let mesh = self.kernel.mesh_step(path, deflection)?;
        encode_mesh(&mesh)
    }

    /// Handles a request given as JSON and answers in JSON. This is the whole
    /// surface a transport needs.
    pub fn handle_json(&mut self, request: &str) -> String {
        let response = match parse_request(request) {
            Ok(request) => self.handle(request),
            Err(error) => failure(error),
        };
        serde_json::to_string(&response).unwrap_or_else(|error| {
            format!(r#"{{"ok":false,"error":{{"kind":"internal","message":"{error}"}}}}"#)
        })
    }

    /// Handles one request.
    pub fn handle(&mut self, request: Request) -> Response {
        let changes_document = matches!(
            request,
            Request::NewDocument { .. }
                | Request::Open { .. }
                | Request::Execute { .. }
                | Request::Undo
                | Request::Redo
        );
        if changes_document {
            self.rollback = None;
        }
        let rolls_back = matches!(request, Request::Rollback { .. });
        let result = match request {
            Request::Rollback { to } => {
                let count = self.session.document().features().len();
                self.rollback = to.filter(|n| *n < count);
                Ok(())
            }
            Request::NewDocument {
                name,
                discard_changes,
            } => self.new_document(&name, discard_changes),
            Request::Open {
                path,
                discard_changes,
            } => self.open(path, discard_changes),
            Request::Save => self.save(),
            Request::SaveAs { path } => {
                // Adopt the new path only once saving to it has worked.
                let previous = self.path.replace(path);
                let result = self.save();
                if result.is_err() {
                    self.path = previous;
                }
                result
            }
            Request::Execute { command, args } => self
                .registry
                .build(&command, &args)
                .and_then(|command| self.session.execute(command))
                .map_err(ApiError::from),
            Request::Undo => self.session.undo().map_err(ApiError::from),
            Request::Redo => self.session.redo().map_err(ApiError::from),
            Request::State => Ok(()),
            Request::Commands => {
                let specs: Vec<_> = self.registry.specs().collect();
                return match serde_json::to_value(specs) {
                    Ok(value) => success(value),
                    Err(error) => failure(ApiError::new("internal", error.to_string())),
                };
            }
            Request::Post {
                post,
                definition,
                machine,
                setup,
                setups,
                override_problems,
                prove_out,
                path,
            } => {
                let post = PostRef {
                    id: post,
                    definition,
                };
                return match self.post(
                    &post,
                    machine.as_deref(),
                    setup,
                    setups.as_deref(),
                    Output {
                        path: path.as_deref(),
                        prove_out,
                    },
                    override_problems,
                ) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::Simulate { setup, cell, after } => {
                return match self.simulate_setup(setup, cell, after.as_deref()) {
                    Ok(sim) => {
                        let report = self.keep_simulation(sim);
                        success(serde_json::json!({
                        "collisions": report.collisions,
                        "gouges": report.gouges,
                        "left": report.left,
                        "left_at": report.left_at,
                        "removed": report.removed,
                        "cut_length": report.cut_length,
                        "rapid_length": report.rapid_length,
                        "cut_minutes": report.cut_minutes,
                        "grid": report.grid,
                        }))
                    }
                    Err(error) => failure(error),
                };
            }
            Request::Verify {
                post,
                definition,
                machine,
                setups,
                cell,
                path,
            } => {
                let post = PostRef {
                    id: post,
                    definition,
                };
                return match self.verify(
                    &post,
                    machine.as_deref(),
                    setups.as_deref(),
                    cell,
                    path.as_deref(),
                ) {
                    Ok(report) => match serde_json::to_value(&report) {
                        Ok(value) => {
                            // Kept for the post that usually follows.
                            if cell.is_none() {
                                let ids: Vec<u32> = setups.unwrap_or_else(|| {
                                    let cam = self.session.document().cam();
                                    cam.setups.iter().map(|s| s.id).collect()
                                });
                                let key = format!(
                                    "{}|{}|{:?}|{ids:?}",
                                    self.revision,
                                    post.key(),
                                    machine.as_deref()
                                );
                                self.verified = Some((key, report));
                            }
                            success(value)
                        }
                        Err(error) => failure(ApiError::new("internal", error.to_string())),
                    },
                    Err(error) => failure(error),
                };
            }
            Request::SetupSheet {
                post,
                definition,
                machine,
                setups,
                path,
            } => {
                let id = match (&post, machine.as_deref().map(shipped_machine)) {
                    (Some(id), _) => Ok(id.clone()),
                    (None, Some(Ok(m))) => Ok(m.machine.post),
                    (None, Some(Err(error))) => Err(error),
                    (None, None) => Ok(String::from("fanuc-mill")),
                };
                return match id.and_then(|id| {
                    let post = PostRef { id, definition };
                    self.setup_sheet(
                        &post,
                        machine.as_deref(),
                        setups.as_deref(),
                        path.as_deref(),
                    )
                }) {
                    Ok(sheet) => success(serde_json::json!({
                        "html": sheet.html,
                        "setups": sheet.setups,
                    })),
                    Err(error) => failure(error),
                };
            }
            Request::Export { path } => {
                return match self.export(&path) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::ExportDxf {
                path,
                sketch,
                section,
            } => {
                return match self.export_dxf(&path, sketch, section) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::ImportDxf { path, plane, name } => {
                return match self.import_dxf(&path, plane, name) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::ImportStep {
                path,
                operation,
                name,
            } => {
                return match self.import_step(&path, operation, name) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::ImportMesh { path, name } => {
                return match self.import_mesh(&path, name) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::PostSource { post } => {
                return match PostRef::shipped_text(&post) {
                    Some(text) => success(serde_json::json!({ "text": text })),
                    None => failure(ApiError::new(
                        "unknown_post",
                        format!("there is no shipped post {post:?}"),
                    )),
                };
            }
            Request::PostPreview {
                definition,
                setup,
                setups,
            } => {
                return match self.post_preview(definition, setup, setups.as_deref()) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
                };
            }
            Request::ReadPost { path } => {
                return match std::fs::read_to_string(&path) {
                    Ok(text) => match cndt_post::Post::from_toml(&text) {
                        Ok(_) => success(serde_json::json!({ "text": text })),
                        Err(e) => failure(ApiError::new(
                            "post_definition",
                            format!("{} is not a valid post: {e}", path.display()),
                        )),
                    },
                    Err(e) => failure(ApiError::new(
                        "io",
                        format!("cannot read {}: {e}", path.display()),
                    )),
                };
            }
            Request::WritePost { definition, path } => {
                if let Err(e) = cndt_post::Post::from_toml(&definition) {
                    return failure(ApiError::new("post_definition", e.to_string()));
                }
                return match std::fs::write(&path, definition) {
                    Ok(()) => success(serde_json::json!({})),
                    Err(e) => failure(ApiError::new(
                        "io",
                        format!("cannot write {}: {e}", path.display()),
                    )),
                };
            }
            Request::Detect { setup } => {
                return match cndt_process_mill::detect(
                    self.session.document(),
                    &self.rebuilt,
                    self.kernel.as_ref(),
                    setup,
                ) {
                    Ok(found) => success(serde_json::json!({
                        "operations": found.operations,
                        "notes": found.notes,
                    })),
                    Err(e) => failure(ApiError::new("cam", e.to_string())),
                };
            }
            Request::ProjectThumbnail { path } => {
                return match cndt_format::open(&path) {
                    Ok(project) => success(serde_json::json!({
                        "png_base64": project
                            .extensions()
                            .get(thumbnail::ENTRY)
                            .map(|png| thumbnail::base64(png)),
                    })),
                    Err(error) => failure(error.into()),
                };
            }
            Request::StepInfo { path } => {
                return match self.kernel.import_step(&path) {
                    Ok(summary) => match serde_json::to_value(summary) {
                        Ok(value) => success(value),
                        Err(error) => failure(ApiError::new("internal", error.to_string())),
                    },
                    Err(error) => failure(error.into()),
                };
            }
        };
        if result.is_ok() && (changes_document || rolls_back) {
            self.refresh();
        }
        match result {
            Ok(()) => match self.state() {
                Ok(state) => success(state),
                Err(error) => failure(error),
            },
            Err(error) => failure(error),
        }
    }

    /// The current state, as JSON.
    ///
    /// # Errors
    ///
    /// Only if the state cannot be encoded, which would be a bug.
    pub fn state(&self) -> Result<Value, ApiError> {
        let view = StateView {
            document: self.session.document(),
            path: self.path.as_ref().map(|path| path.display().to_string()),
            dirty: self.session.is_dirty(),
            undo: self.session.undo_label(),
            redo: self.session.redo_label(),
            freedom: self
                .session
                .document()
                .sketches()
                .iter()
                .map(|(id, record)| (*id, record.sketch.freedom()))
                .collect(),
            feature_status: &self.rebuilt.status,
            sketch_frames: &self.rebuilt.frames,
            body: self.rebuilt.properties.map(|p| BodyView {
                volume: p.volume,
                bounds: p.bounds,
                faces: self
                    .rebuilt
                    .faces
                    .iter()
                    .map(|names| names.iter().map(ToString::to_string).collect())
                    .collect(),
                edges: self
                    .rebuilt
                    .body
                    .as_ref()
                    .and_then(|body| self.kernel.edges(body).ok())
                    .unwrap_or_default(),
            }),
            body_revision: self.revision,
            rollback: self.rollback,
            tool_assemblies: self
                .session
                .document()
                .cam()
                .tools
                .iter()
                .map(|(number, tool)| {
                    (
                        *number,
                        ToolAssemblyView {
                            assembly: tool.assembly(),
                            measured: tool.assembly.is_some(),
                        },
                    )
                })
                .collect(),
            references: self
                .rebuilt
                .references
                .iter()
                .map(|(id, reference)| {
                    (
                        *id,
                        ReferenceView {
                            triangles: reference.mesh.triangle_count(),
                            bounds: reference.bounds,
                            size: reference.bounds.size(),
                        },
                    )
                })
                .collect(),
        };
        serde_json::to_value(view).map_err(|error| ApiError::new("internal", error.to_string()))
    }

    fn guard_unsaved(&self, discard_changes: bool) -> Result<(), ApiError> {
        if self.session.is_dirty() && !discard_changes {
            Err(ApiError::new(
                "unsaved_changes",
                "the document has unsaved changes; save it first or pass discard_changes",
            ))
        } else {
            Ok(())
        }
    }

    fn new_document(&mut self, name: &str, discard_changes: bool) -> Result<(), ApiError> {
        self.guard_unsaved(discard_changes)?;
        let mut document = Document::default();
        document
            .set_name(name)
            .map_err(|error| ApiError::new("rejected", error.to_string()))?;
        self.session = Session::new(document);
        self.path = None;
        self.extensions.clear();
        Ok(())
    }

    fn open(&mut self, path: PathBuf, discard_changes: bool) -> Result<(), ApiError> {
        self.guard_unsaved(discard_changes)?;
        let project = cndt_format::open(&path)?;
        self.extensions = project.extensions().clone();
        self.session = Session::new(project.document);
        self.path = Some(path);
        Ok(())
    }

    /// The part's thumbnail (decision 0025), or `None` without a solid or
    /// when it cannot be drawn: a picture is never worth failing a save for.
    fn thumbnail_png(&self) -> Option<Vec<u8>> {
        let body = self.rebuilt.body.as_ref()?;
        let mesh = self.kernel.mesh(body, self.deflection()).ok()?;
        thumbnail::render(&mesh)
    }

    fn save(&mut self) -> Result<(), ApiError> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| ApiError::new("no_path", "the document has no file yet; use save_as"))?;
        let mut project = Project::new(self.session.document().clone());
        for (name, data) in &self.extensions {
            // The picture is made again from the part on every save.
            if name != thumbnail::ENTRY {
                project.set_extension(name, data.clone())?;
            }
        }
        if let Some(png) = self.thumbnail_png() {
            project.set_extension(thumbnail::ENTRY, png)?;
        }
        cndt_format::save(&project, &path)?;
        self.session.mark_saved();
        Ok(())
    }
}

/// The fields each method accepts, besides `method` itself.
const FIELDS: &[(&str, &[&str])] = &[
    ("new_document", &["name", "discard_changes"]),
    ("open", &["path", "discard_changes"]),
    ("save", &[]),
    ("save_as", &["path"]),
    ("execute", &["command", "args"]),
    ("undo", &[]),
    ("redo", &[]),
    ("state", &[]),
    ("commands", &[]),
    ("project_thumbnail", &["path"]),
    ("step_info", &["path"]),
    (
        "post",
        &[
            "post",
            "definition",
            "machine",
            "setup",
            "setups",
            "override",
            "prove_out",
            "path",
        ],
    ),
    ("detect", &["setup"]),
    ("simulate", &["setup", "cell", "after"]),
    ("rollback", &["to"]),
    (
        "verify",
        &["post", "definition", "machine", "setups", "cell", "path"],
    ),
    (
        "setup_sheet",
        &["post", "definition", "machine", "setups", "path"],
    ),
    ("export", &["path"]),
    ("import_dxf", &["path", "plane", "name"]),
    ("import_step", &["path", "operation", "name"]),
    ("import_mesh", &["path", "name"]),
    ("export_dxf", &["path", "sketch", "section"]),
    ("post_source", &["post"]),
    ("post_preview", &["definition", "setup", "setups"]),
    ("read_post", &["path"]),
    ("write_post", &["definition", "path"]),
];

/// Parses a request, refusing fields the method does not accept, so a typo
/// such as `discard_change` fails loudly instead of being ignored. (Serde's
/// `deny_unknown_fields` does not cover internally tagged enums.)
fn parse_request(text: &str) -> Result<Request, ApiError> {
    let bad = |message: String| ApiError::new("bad_request", message);
    let value: Value = serde_json::from_str(text).map_err(|error| bad(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| bad(String::from("a request must be a JSON object")))?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| bad(String::from("a request needs a \"method\" string")))?;
    let allowed = FIELDS
        .iter()
        .find(|(name, _)| *name == method)
        .map(|(_, fields)| *fields)
        .ok_or_else(|| bad(format!("unknown method {method:?}")))?;
    if let Some(key) = object
        .keys()
        .find(|key| key.as_str() != "method" && !allowed.contains(&key.as_str()))
    {
        return Err(bad(format!("{method} does not accept the field {key:?}")));
    }
    serde_json::from_value(value).map_err(|error| bad(error.to_string()))
}

/// Where a posted program goes, and whether it is a prove-out.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Output<'a> {
    /// A file to write it to.
    path: Option<&'a Path>,
    /// Every Z raised this far, for an air cut.
    prove_out: Option<f64>,
}

/// How far above the stock a prove-out's lowest point must stay, in mm.
const PROVE_OUT_CLEARANCE: f64 = 5.0;

fn encode_mesh(mesh: &TriangleMesh) -> Result<Vec<u8>, ApiError> {
    let too_large = || ApiError::new("kernel", "the mesh is too large to send");
    let vertices = u32::try_from(mesh.vertex_count()).map_err(|_| too_large())?;
    let triangles = u32::try_from(mesh.triangle_count()).map_err(|_| too_large())?;
    let mut bytes = Vec::with_capacity(
        20 + 4 * (mesh.positions.len() + mesh.indices.len() + mesh.triangle_faces.len()),
    );
    bytes.extend_from_slice(MESH_MAGIC);
    bytes.extend_from_slice(&vertices.to_le_bytes());
    bytes.extend_from_slice(&triangles.to_le_bytes());
    bytes.extend_from_slice(&mesh.face_count.to_le_bytes());
    for value in &mesh.positions {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in mesh.indices.iter().chain(&mesh.triangle_faces) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

fn success(result: Value) -> Response {
    Response {
        ok: true,
        result: Some(result),
        error: None,
    }
}

fn failure(error: ApiError) -> Response {
    Response {
        ok: false,
        result: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(api: &mut Api, request: Value) -> Value {
        serde_json::from_str(&api.handle_json(&request.to_string())).unwrap()
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cndt-api-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn edit_undo_redo_through_json() {
        let mut api = Api::new();
        let set = json!({"method": "execute", "command": "parameter.set",
                         "args": {"name": "thickness", "value": 12, "unit": "mm"}});
        let reply = call(&mut api, set);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(
            reply["result"]["document"]["parameters"]["thickness"]["value"],
            12.0
        );
        assert_eq!(reply["result"]["dirty"], true);
        assert_eq!(reply["result"]["undo"], "Set thickness");

        let reply = call(&mut api, json!({"method": "undo"}));
        assert!(reply["result"]["document"]["parameters"]["thickness"].is_null());
        let reply = call(&mut api, json!({"method": "redo"}));
        assert_eq!(
            reply["result"]["document"]["parameters"]["thickness"]["unit"],
            "millimetre"
        );
    }

    #[test]
    fn sketch_through_json_with_freedom_and_conflicts() {
        let mut api = Api::new();
        let exec = |api: &mut Api, command: &str, args: Value| {
            call(
                api,
                json!({"method": "execute", "command": command, "args": args}),
            )
        };
        let reply = exec(&mut api, "sketch.create", json!({"plane": "xy"}));
        assert_eq!(
            reply["result"]["document"]["sketches"]["0"]["name"],
            "Sketch1"
        );
        let reply = exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 90, "y": 50}}),
        );
        assert_eq!(reply["result"]["freedom"]["0"]["dof"], 4);
        assert_eq!(reply["result"]["freedom"]["0"]["status"], "under");
        // Bottom-left corner fixed, then width and height: fully constrained.
        exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "fix", "point": 0}}),
        );
        exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "length", "line": 4, "value": 96}}),
        );
        let reply = exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "length", "line": 5, "value": 56}}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["freedom"]["0"]["status"], "fully");
        let corner = &reply["result"]["document"]["sketches"]["0"]["sketch"]["entities"]["2"]["geometry"]
            ["at"];
        assert!(
            (corner["x"].as_f64().unwrap() - 96.0).abs() < 1e-7,
            "{corner}"
        );
        assert_eq!(reply["result"]["undo"], "Add constraint");
        // A conflicting dimension names the constraints it fights.
        let reply = exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "length", "line": 6, "value": 80}}),
        );
        assert_eq!(reply["error"]["kind"], "conflict", "{reply}");
        let ids = reply["error"]["details"]["constraints"].as_array().unwrap();
        assert_eq!(ids.first(), Some(&json!(7)), "the new one first");
        assert!(ids.contains(&json!(5)), "the 96 mm width: {ids:?}");
        // A redundant one names what already implies it.
        let reply = exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "parallel", "a": 4, "b": 6}}),
        );
        assert_eq!(reply["error"]["kind"], "redundant", "{reply}");
        // Changing a dimension moves the geometry; undo puts it back.
        exec(
            &mut api,
            "sketch.set_dimension",
            json!({"sketch": 0, "constraint": 5, "value": 100}),
        );
        let reply = call(&mut api, json!({"method": "undo"}));
        let corner = &reply["result"]["document"]["sketches"]["0"]["sketch"]["entities"]["2"]["geometry"]
            ["at"];
        assert!(
            (corner["x"].as_f64().unwrap() - 96.0).abs() < 1e-7,
            "{corner}"
        );
    }

    #[test]
    fn extrude_and_cut_through_json_with_named_faces_and_a_mesh() {
        let mut api = Api::new();
        let exec = |api: &mut Api, command: &str, args: Value| {
            call(
                api,
                json!({"method": "execute", "command": command, "args": args}),
            )
        };
        exec(&mut api, "sketch.create", json!({"plane": "xy"}));
        exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 96, "y": 56}}),
        );
        let reply = exec(
            &mut api,
            "feature.extrude",
            json!({"sketch": 0, "distance": 20}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        let state = &reply["result"];
        assert_eq!(state["feature_status"]["0"]["state"], "ok");
        let volume = state["body"]["volume"].as_f64().unwrap();
        assert!((volume - 96.0 * 56.0 * 20.0).abs() < 1e-6);
        assert_eq!(state["body"]["faces"].as_array().unwrap().len(), 6);
        assert!(state["body"]["faces"].to_string().contains("f0/side/e4"));
        let revision = state["body_revision"].as_u64().unwrap();
        // Reading the state does not rebuild.
        let again = call(&mut api, json!({"method": "state"}));
        assert_eq!(again["result"]["body_revision"].as_u64(), Some(revision));
        let mesh = api.body_mesh().unwrap();
        assert_eq!(&mesh[..8], MESH_MAGIC);
        let edges = api.body_edges().unwrap();
        assert_eq!(&edges[..8], EDGES_MAGIC);
        assert_eq!(u32::from_le_bytes(edges[8..12].try_into().unwrap()), 12);
        assert_eq!(state["body"]["edges"].as_array().unwrap().len(), 12);

        // A pocket cut from a second sketch.
        exec(&mut api, "sketch.create", json!({"plane": "xy"}));
        exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 1, "from": {"x": 28, "y": 18}, "to": {"x": 68, "y": 38}}),
        );
        let reply = exec(
            &mut api,
            "feature.extrude",
            json!({"sketch": 1, "distance": 8, "operation": "cut"}),
        );
        let volume = reply["result"]["body"]["volume"].as_f64().unwrap();
        assert!(
            (volume - (96.0 * 56.0 * 20.0 - 40.0 * 20.0 * 8.0)).abs() < 1e-6,
            "{volume}"
        );
        assert_eq!(reply["result"]["undo"], "Extrude cut");
        // The same pocket, sketched on the top face and cut down into it.
        call(&mut api, json!({"method": "undo"}));
        let top = reply["result"]["body"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|names| names.as_array().unwrap().iter())
            .filter_map(Value::as_str)
            .find(|name| name.starts_with("f0/end/"))
            .unwrap()
            .to_owned();
        let reply = exec(&mut api, "sketch.create", json!({"face": top}));
        assert_eq!(
            reply["result"]["sketch_frames"]["2"]["frame"]["origin"]["z"], 20.0,
            "{reply}"
        );
        exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 2, "from": {"x": 28, "y": 18}, "to": {"x": 68, "y": 38}}),
        );
        let reply = exec(
            &mut api,
            "feature.extrude",
            json!({"sketch": 2, "distance": -8, "operation": "cut"}),
        );
        assert_eq!(reply["result"]["body"]["bounds"]["max"]["z"], 20.0);
        let volume = reply["result"]["body"]["volume"].as_f64().unwrap();
        assert!(
            (volume - (96.0 * 56.0 * 20.0 - 40.0 * 20.0 * 8.0)).abs() < 1e-6,
            "{volume}"
        );
        // Deleting the sketch a feature uses is refused with a reason.
        let reply = exec(&mut api, "sketch.delete", json!({"sketch": 2}));
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Extrude2"),
            "{reply}"
        );
        // Undo the cut: the block is whole again.
        let reply = call(&mut api, json!({"method": "undo"}));
        let volume = reply["result"]["body"]["volume"].as_f64().unwrap();
        assert!((volume - 96.0 * 56.0 * 20.0).abs() < 1e-6);
    }

    #[test]
    fn tapped_and_counterbored_holes_through_json() {
        let mut api = Api::new();
        let exec = |api: &mut Api, command: &str, args: Value| {
            call(
                api,
                json!({"method": "execute", "command": command, "args": args}),
            )
        };
        exec(&mut api, "sketch.create", json!({"plane": "xy"}));
        exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 96, "y": 56}}),
        );
        exec(
            &mut api,
            "feature.extrude",
            json!({"sketch": 0, "distance": 20}),
        );
        let state = call(&mut api, json!({"method": "state"}));
        let top = state["result"]["body"]["faces"].to_string();
        let top = top
            .split('"')
            .find(|t| t.starts_with("f0/end/"))
            .unwrap()
            .to_owned();
        exec(&mut api, "sketch.create", json!({"face": top}));
        exec(
            &mut api,
            "sketch.add_point",
            json!({"sketch": 1, "at": {"x": 20, "y": 8}}),
        );
        exec(
            &mut api,
            "sketch.add_point",
            json!({"sketch": 1, "at": {"x": 76, "y": 8}}),
        );
        let reply = exec(
            &mut api,
            "feature.hole",
            json!({"sketch": 1, "thread": "M6", "depth": 15, "thread_depth": 12}),
        );
        assert_eq!(
            reply["result"]["feature_status"]["1"]["state"], "ok",
            "{reply}"
        );
        let hole = &reply["result"]["document"]["features"][1]["feature"];
        assert_eq!(hole["spec"]["thread"]["size"], "M6x1.0");
        assert_eq!(hole["spec"]["diameter"], 5.0, "the tap drill");
        assert!(
            reply["result"]["body"]["faces"]
                .to_string()
                .contains("/hole/e0/tip")
        );
        // Make them counterbored clearance holes instead.
        let reply = exec(
            &mut api,
            "feature.edit_hole",
            json!({"feature": 1, "diameter": 5.5, "depth": "through", "counterbore_diameter": 9.5, "counterbore_depth": 5.4}),
        );
        assert!(
            reply["result"]["body"]["faces"]
                .to_string()
                .contains("/hole/e0/counterbore_floor"),
            "{reply}"
        );
        // Chamfer the top face's edges: the outline and both counterbores.
        let reply = exec(
            &mut api,
            "feature.chamfer",
            json!({"faces": [top], "distance": 0.5}),
        );
        assert_eq!(
            reply["result"]["feature_status"]["2"]["state"], "ok",
            "{reply}"
        );
        assert_eq!(reply["result"]["undo"], "Chamfer");
        let reply = exec(
            &mut api,
            "feature.edit_blend",
            json!({"feature": 2, "size": 0.8}),
        );
        assert_eq!(
            reply["result"]["feature_status"]["2"]["state"], "ok",
            "{reply}"
        );
        call(&mut api, json!({"method": "undo"}));
        call(&mut api, json!({"method": "undo"}));
        // A bad edit is refused and changes nothing.
        let reply = exec(
            &mut api,
            "feature.edit_hole",
            json!({"feature": 1, "diameter": 12, "depth": "through", "counterbore_diameter": 9.5, "counterbore_depth": 5.4}),
        );
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("wider than the hole"),
            "{reply}"
        );
    }

    #[test]
    fn dimensions_follow_parameters() {
        let mut api = Api::new();
        let exec = |api: &mut Api, command: &str, args: Value| {
            call(
                api,
                json!({"method": "execute", "command": command, "args": args}),
            )
        };
        exec(
            &mut api,
            "parameter.set",
            json!({"name": "width", "value": 96, "unit": "mm"}),
        );
        exec(&mut api, "sketch.create", json!({"plane": "xy"}));
        exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 90, "y": 50}}),
        );
        exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "fix", "point": 0}}),
        );
        exec(
            &mut api,
            "sketch.constrain",
            json!({"sketch": 0, "constraint": {"type": "length", "line": 4, "value": 90}}),
        );
        let reply = exec(
            &mut api,
            "sketch.set_dimension",
            json!({"sketch": 0, "constraint": 5, "value": "width / 2"}),
        );
        let sketch = &reply["result"]["document"]["sketches"]["0"];
        assert_eq!(sketch["expressions"]["5"], "width / 2", "{reply}");
        assert_eq!(sketch["sketch"]["constraints"]["5"]["value"], 48.0);
        let reply = exec(
            &mut api,
            "parameter.set",
            json!({"name": "width", "value": 120, "unit": "mm"}),
        );
        assert_eq!(
            reply["result"]["document"]["sketches"]["0"]["sketch"]["constraints"]["5"]["value"],
            60.0
        );
        let reply = exec(
            &mut api,
            "sketch.set_dimension",
            json!({"sketch": 0, "constraint": 5, "value": "depth"}),
        );
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no parameter named depth"),
            "{reply}"
        );
        // Typing a number drops the expression.
        let reply = exec(
            &mut api,
            "sketch.set_dimension",
            json!({"sketch": 0, "constraint": 5, "value": 70}),
        );
        assert!(
            reply["result"]["document"]["sketches"]["0"]
                .get("expressions")
                .is_none(),
            "{reply}"
        );
    }

    #[test]
    fn errors_carry_a_stable_kind() {
        let mut api = Api::new();
        let cases = [
            (json!("not an object"), "bad_request"),
            (json!({"method": "fly"}), "bad_request"),
            (json!({"method": "state", "extra": 1}), "bad_request"),
            (
                json!({"method": "open", "path": "x.cndt", "discard_change": true}),
                "bad_request",
            ),
            (
                json!({"method": "execute", "command": "nope"}),
                "unknown_command",
            ),
            (
                json!({"method": "execute", "command": "parameter.remove", "args": {}}),
                "invalid_arguments",
            ),
            (
                json!({"method": "execute", "command": "parameter.remove", "args": {"name": "x"}}),
                "rejected",
            ),
            (json!({"method": "undo"}), "nothing_to_undo"),
            (json!({"method": "save"}), "no_path"),
        ];
        for (request, kind) in cases {
            let reply = call(&mut api, request.clone());
            assert_eq!(reply["ok"], false, "{request}");
            assert_eq!(reply["error"]["kind"], kind, "{request}: {reply}");
        }
    }

    #[test]
    fn save_open_round_trip_and_unsaved_guard() {
        let path = temp_file("roundtrip.cndt");
        let mut api = Api::new();
        call(
            &mut api,
            json!({"method": "new_document", "name": "Bracket"}),
        );
        call(
            &mut api,
            json!({"method": "execute", "command": "parameter.set",
                              "args": {"name": "width", "value": 80, "unit": "mm"}}),
        );
        let reply = call(&mut api, json!({"method": "save_as", "path": path}));
        assert_eq!(reply["result"]["dirty"], false, "{reply}");

        call(
            &mut api,
            json!({"method": "execute", "command": "document.rename",
                              "args": {"name": "Changed"}}),
        );
        let reply = call(&mut api, json!({"method": "open", "path": path}));
        assert_eq!(reply["error"]["kind"], "unsaved_changes");

        let reply = call(
            &mut api,
            json!({"method": "open", "path": path, "discard_changes": true}),
        );
        assert_eq!(reply["result"]["document"]["name"], "Bracket");
        assert_eq!(
            reply["result"]["document"]["parameters"]["width"]["value"],
            80.0
        );
        assert_eq!(reply["result"]["dirty"], false);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_imported_step_solid_is_kept_in_the_project() {
        let step =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/box-10x20x30.step");
        let path = temp_file("imported.cndt");
        let mut api = Api::new();
        call(
            &mut api,
            json!({"method": "new_document", "name": "Imported"}),
        );
        let reply = call(&mut api, json!({"method": "import_step", "path": step}));
        let result = &reply["result"];
        assert_eq!(result["feature_status"]["0"]["state"], "ok", "{reply}");
        let feature = &result["document"]["features"][0]["feature"];
        assert_eq!(feature["type"], "import");
        assert_eq!(feature["name"], "Import1");
        assert_eq!(feature["source"], "box-10x20x30.step");
        assert!(
            feature.get("data").is_none(),
            "the content never goes to the UI"
        );
        assert!((result["body"]["volume"].as_f64().unwrap() - 6000.0).abs() < 1e-6);
        assert_eq!(result["undo"], "Import box-10x20x30.step");
        call(&mut api, json!({"method": "save_as", "path": path}));

        // Reopened without the original file being read again.
        let mut other = Api::new();
        let reply = call(&mut other, json!({"method": "open", "path": path}));
        assert_eq!(
            reply["result"]["feature_status"]["0"]["state"], "ok",
            "{reply}"
        );
        assert!((reply["result"]["body"]["volume"].as_f64().unwrap() - 6000.0).abs() < 1e-6);

        let reply = call(&mut api, json!({"method": "undo"}));
        assert!(reply["result"]["body"].is_null(), "{reply}");

        // An IGES file of surfaces, cut from the box: sewn into a solid.
        let mut api = Api::new();
        call(&mut api, json!({"method": "new_document", "name": "Iges"}));
        let iges = step.with_file_name("box-10x20x30-surfaces.igs");
        let reply = call(&mut api, json!({"method": "import_step", "path": iges}));
        assert_eq!(
            reply["result"]["feature_status"]["0"]["state"], "ok",
            "{reply}"
        );
        assert_eq!(
            reply["result"]["document"]["features"][0]["feature"]["format"],
            "iges"
        );
        assert!((reply["result"]["body"]["volume"].as_f64().unwrap() - 6000.0).abs() < 1e-3);
        let iges_project = temp_file("iges.cndt");
        call(&mut api, json!({"method": "save_as", "path": iges_project}));
        let reply = call(
            &mut Api::new(),
            json!({"method": "open", "path": iges_project}),
        );
        assert_eq!(
            reply["result"]["feature_status"]["0"]["state"], "ok",
            "{reply}"
        );
        std::fs::remove_file(iges_project).unwrap();

        let text = temp_file("not-step.step");
        std::fs::write(&text, "solid nothing\nendsolid\n").unwrap();
        let reply = call(&mut api, json!({"method": "import_step", "path": text}));
        let message = reply["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("not a STEP or IGES file"), "{reply}");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(text).unwrap();
    }

    #[test]
    fn a_reference_mesh_is_shown_and_measured_but_never_joins_the_part() {
        // A binary STL tetrahedron, 10 × 20 × 30 mm, beside an imported box.
        let mut stl = vec![0_u8; 80];
        let corners: [[f32; 3]; 4] = [
            [50.0, 0.0, 0.0],
            [60.0, 0.0, 0.0],
            [50.0, 20.0, 0.0],
            [50.0, 0.0, 30.0],
        ];
        let faces = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        stl.extend_from_slice(&4_u32.to_le_bytes());
        for face in faces {
            stl.extend_from_slice(&[0_u8; 12]);
            for corner in face {
                for value in corners[corner] {
                    stl.extend_from_slice(&value.to_le_bytes());
                }
            }
            stl.extend_from_slice(&[0, 0]);
        }
        let mesh_path = temp_file("tetra.stl");
        std::fs::write(&mesh_path, &stl).unwrap();
        let step =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/box-10x20x30.step");
        let project = temp_file("reference.cndt");

        let mut api = Api::new();
        call(&mut api, json!({"method": "new_document", "name": "Mesh"}));
        call(&mut api, json!({"method": "import_step", "path": step}));
        let reply = call(
            &mut api,
            json!({"method": "import_mesh", "path": mesh_path}),
        );
        let result = &reply["result"];
        assert_eq!(result["feature_status"]["1"]["state"], "ok", "{reply}");
        assert_eq!(
            result["document"]["features"][1]["feature"]["type"],
            "reference"
        );
        assert_eq!(
            result["document"]["features"][1]["feature"]["name"],
            "Mesh1"
        );
        assert_eq!(result["references"]["1"]["triangles"], 4);
        assert_eq!(result["references"]["1"]["size"], json!([10.0, 20.0, 30.0]));
        // The part is the box alone.
        assert!((result["body"]["volume"].as_f64().unwrap() - 6000.0).abs() < 1e-6);
        assert_eq!(result["undo"], "Mesh tetra.stl");
        assert_eq!(api.reference_mesh(1).unwrap().len(), 20 + 4 * (36 + 12 + 4));
        assert_eq!(api.reference_mesh(0).unwrap_err().kind, "unknown_feature");

        call(&mut api, json!({"method": "save_as", "path": project}));
        let mut other = Api::new();
        let reply = call(&mut other, json!({"method": "open", "path": project}));
        assert_eq!(
            reply["result"]["references"]["1"]["triangles"], 4,
            "{reply}"
        );

        std::fs::write(&mesh_path, "ISO-10303-21;").unwrap();
        let reply = call(
            &mut api,
            json!({"method": "import_mesh", "path": mesh_path}),
        );
        let message = reply["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("tetra.stl: the file is not STL"),
            "{reply}"
        );
        std::fs::remove_file(project).unwrap();
        std::fs::remove_file(mesh_path).unwrap();
    }

    #[test]
    fn saving_writes_a_thumbnail_that_can_be_read_without_opening() {
        let path = temp_file("thumbnail.cndt");
        let mut api = Api::new();
        // No solid yet: nothing is stored.
        call(&mut api, json!({"method": "save_as", "path": path}));
        let none = call(
            &mut api,
            json!({"method": "project_thumbnail", "path": path}),
        );
        assert_eq!(none["ok"], true, "{none}");
        assert!(none["result"]["png_base64"].is_null());

        let exec = |api: &mut Api, command: &str, args: Value| {
            let reply = call(
                api,
                json!({"method": "execute", "command": command, "args": args}),
            );
            assert_eq!(reply["ok"], true, "{reply}");
        };
        exec(&mut api, "sketch.create", json!({"plane": "xy"}));
        exec(
            &mut api,
            "sketch.add_rectangle",
            json!({"sketch": 0, "from": {"x": 0, "y": 0}, "to": {"x": 40, "y": 20}}),
        );
        exec(
            &mut api,
            "feature.extrude",
            json!({"sketch": 0, "distance": 10}),
        );
        call(&mut api, json!({"method": "save"}));

        let reply = call(
            &mut api,
            json!({"method": "project_thumbnail", "path": path}),
        );
        let encoded = reply["result"]["png_base64"].as_str().unwrap().to_owned();
        assert!(
            encoded.starts_with("iVBORw0KGgo"),
            "a PNG signature: {encoded:.20}"
        );
        let stored = cndt_format::open(&path).unwrap();
        let png = stored.extensions().get(thumbnail::ENTRY).unwrap();
        assert_eq!(thumbnail::base64(png), encoded);

        // Saving again makes the same picture; opening and saving keeps one entry.
        call(&mut api, json!({"method": "save"}));
        let again = call(
            &mut api,
            json!({"method": "project_thumbnail", "path": path}),
        );
        assert_eq!(again["result"]["png_base64"], encoded);
        // A file that is not a project is refused with a readable error.
        let bad = call(
            &mut api,
            json!({"method": "project_thumbnail", "path": "missing.cndt"}),
        );
        assert_eq!(bad["ok"], false);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn extension_data_survives_open_and_save() {
        let path = temp_file("extensions.cndt");
        let mut project = Project::new(Document::new("Ext"));
        project.set_extension("future/blob", vec![7, 8, 9]).unwrap();
        cndt_format::save(&project, &path).unwrap();

        let mut api = Api::new();
        call(&mut api, json!({"method": "open", "path": path}));
        call(
            &mut api,
            json!({"method": "execute", "command": "document.rename",
                              "args": {"name": "Ext2"}}),
        );
        call(&mut api, json!({"method": "save"}));

        let back = cndt_format::open(&path).unwrap();
        assert_eq!(back.document.name(), "Ext2");
        assert_eq!(back.extensions().get("future/blob"), Some(&vec![7, 8, 9]));
        std::fs::remove_file(path).unwrap();
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn step_info_and_binary_preview() {
        let mut api = Api::new();
        let path = fixture("box-10x20x30.step");
        let reply = call(&mut api, json!({"method": "step_info", "path": path}));
        assert_eq!(reply["result"]["faces"], 6, "{reply}");
        assert_eq!(reply["result"]["bounds"]["max"]["z"], 30.0);

        let bytes = api.step_preview(&path).unwrap();
        assert_eq!(&bytes[..8], MESH_MAGIC);
        let word = |offset: usize| {
            u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };
        let (vertices, triangles, faces) = (word(8) as usize, word(12) as usize, word(16));
        assert_eq!(triangles, 12);
        assert_eq!(faces, 6);
        assert_eq!(
            bytes.len(),
            20 + 4 * (vertices * 3 + triangles * 3 + triangles)
        );

        let reply = call(
            &mut api,
            json!({"method": "step_info", "path": "missing.step"}),
        );
        assert_eq!(reply["error"]["kind"], "unreadable");
        assert_eq!(
            api.step_preview(Path::new("missing.step"))
                .unwrap_err()
                .kind,
            "unreadable"
        );
    }

    #[test]
    fn a_failed_save_as_keeps_the_old_path() {
        let good = temp_file("keep-path.cndt");
        let mut api = Api::new();
        call(&mut api, json!({"method": "save_as", "path": good}));
        let bad = temp_file("no-such-dir").join("deeper").join("x.cndt");
        let reply = call(&mut api, json!({"method": "save_as", "path": bad}));
        assert_eq!(reply["error"]["kind"], "io", "{reply}");
        let reply = call(&mut api, json!({"method": "state"}));
        assert_eq!(reply["result"]["path"], good.display().to_string());
        std::fs::remove_file(good).unwrap();
    }

    #[test]
    fn commands_are_described() {
        let mut api = Api::new();
        let reply = call(&mut api, json!({"method": "commands"}));
        let names: Vec<&str> = reply["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|spec| spec["name"].as_str())
            .collect();
        assert_eq!(names.len(), 44, "{names:?}");
        for expected in [
            "document.rename",
            "parameter.set",
            "sketch.create",
            "sketch.constrain",
        ] {
            assert!(names.contains(&expected), "{expected} missing: {names:?}");
        }
        assert!(names.is_sorted(), "listed in name order");
    }
}
