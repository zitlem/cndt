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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cndt_engine::{CommandError, Registry, Session};
use cndt_format::{FormatError, Project};
use cndt_geom::TriangleMesh;
use cndt_kernel::{Kernel, KernelError, OpenCascade};
use cndt_model::sketch::Freedom;
use cndt_model::{
    Document, FeatureId, FeatureStatus, FrameStatus, ModelError, Rebuilt, SketchError, SketchId,
    rebuild,
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
    /// Summarise a STEP file's topology and size without importing it.
    StepInfo {
        /// The STEP file.
        path: PathBuf,
    },
    /// G-code for one stored setup (decision 0014), through a shipped
    /// post. Replies `{gcode, notes}`; with `path`, also writes the G-code
    /// there.
    Post {
        /// `fanuc-mill` or `grbl`.
        post: String,
        /// The setup's id; the first setup if left out.
        #[serde(default)]
        setup: Option<u32>,
        /// A file to write the program to.
        #[serde(default)]
        path: Option<PathBuf>,
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
];

/// The first bytes of a toolpath preview buffer.
pub const TOOLPATH_MAGIC: &[u8; 8] = b"CNDTTPH1";

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
        }
    }

    /// Rebuilds the part from the timeline after the document changed.
    fn refresh(&mut self) {
        self.rebuilt = rebuild(self.session.document(), self.kernel.as_ref());
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
    fn post(&self, id: &str, setup: Option<u32>, path: Option<&Path>) -> Result<Value, ApiError> {
        let text = SHIPPED_POSTS
            .iter()
            .find(|(name, _)| *name == id)
            .map(|(_, text)| *text)
            .ok_or_else(|| {
                let names: Vec<&str> = SHIPPED_POSTS.iter().map(|(name, _)| *name).collect();
                ApiError::new(
                    "unknown_post",
                    format!("there is no post {id:?}; use one of {}", names.join(", ")),
                )
            })?;
        let post = cndt_post::Post::from_toml(text)
            .map_err(|e| ApiError::new("internal", e.to_string()))?;
        let planned = cndt_process_mill::plan_setup(
            self.session.document(),
            &self.rebuilt,
            self.kernel.as_ref(),
            setup,
        )
        .map_err(|e| ApiError::new("cam", e.to_string()))?;
        let gcode = post
            .run(&planned.program, env!("CARGO_PKG_VERSION"))
            .map_err(|e| ApiError::new("post", e.to_string()))?;
        if let Some(path) = path {
            std::fs::write(path, &gcode).map_err(|e| {
                ApiError::new("io", format!("cannot write {}: {e}", path.display()))
            })?;
        }
        Ok(serde_json::json!({ "gcode": gcode, "notes": planned.notes }))
    }

    /// A setup's toolpath for drawing, in model coordinates. Layout,
    /// little-endian: [`TOOLPATH_MAGIC`]; u32 segment count; then per
    /// segment u8 kind (0 rapid, 1 cut) and f32 x, y, z from, x, y, z to.
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
        let segments = cndt_process_mill::preview(&planned.program, &placed);
        let mut out = Vec::with_capacity(12 + segments.len() * 25);
        out.extend_from_slice(TOOLPATH_MAGIC);
        out.extend_from_slice(
            &u32::try_from(segments.len())
                .map_err(|_| ApiError::new("internal", "toolpath too long"))?
                .to_le_bytes(),
        );
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
        let result = match request {
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
            Request::Post { post, setup, path } => {
                return match self.post(&post, setup, path.as_deref()) {
                    Ok(value) => success(value),
                    Err(error) => failure(error),
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
        if result.is_ok() && changes_document {
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

    fn save(&mut self) -> Result<(), ApiError> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| ApiError::new("no_path", "the document has no file yet; use save_as"))?;
        let mut project = Project::new(self.session.document().clone());
        for (name, data) in &self.extensions {
            project.set_extension(name, data.clone())?;
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
    ("step_info", &["path"]),
    ("post", &["post", "setup", "path"]),
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
        assert_eq!(names.len(), 33, "{names:?}");
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
