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
use std::path::PathBuf;

use cndt_engine::{CommandError, Registry, Session};
use cndt_format::{FormatError, Project};
use cndt_model::Document;
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
}

impl ApiError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<CommandError> for ApiError {
    fn from(error: CommandError) -> Self {
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
}

/// One open document and everything needed to work on it.
#[derive(Debug)]
pub struct Api {
    session: Session,
    registry: Registry,
    path: Option<PathBuf>,
    /// Module data from the opened file, written back unchanged on save.
    extensions: BTreeMap<String, Vec<u8>>,
}

impl Default for Api {
    fn default() -> Self {
        Self::new()
    }
}

impl Api {
    /// Starts with an empty, unsaved document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            session: Session::new(Document::default()),
            registry: Registry::standard(),
            path: None,
            extensions: BTreeMap::new(),
        }
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
                self.path = Some(path);
                self.save()
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
        };
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
        assert_eq!(
            names,
            ["document.rename", "parameter.remove", "parameter.set"]
        );
    }
}
