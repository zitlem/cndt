// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 3: reading and writing CNDT project files (`.cndt`).
//!
//! A project file is a ZIP container:
//!
//! | Entry | Content |
//! | --- | --- |
//! | `manifest.json` | Format name, schema version, writing app version, units |
//! | `design/document.json` | The design document |
//! | `extensions/...` | Data owned by modules; kept as-is even if unknown |
//!
//! The rules it follows (architecture "Files" section, decision 0009):
//! - Every file has a schema version. A newer file is refused with a clear
//!   message; an older one would be migrated.
//! - The same project always produces the same bytes: fixed timestamps,
//!   fixed permissions, sorted entries, stable JSON.
//! - Saving writes a temporary file and renames it over the target, so a
//!   crash never leaves a half-written project.
//! - Reading is bounded, so a malicious file cannot exhaust memory.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use cndt_model::{Document, ModelError};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// Identifies a CNDT project in its manifest.
pub const FORMAT_NAME: &str = "cndt-project";

/// The schema version this build writes and the newest it can read.
///
/// - 1: name and parameters.
/// - 2: sketches. A version 1 file reads as a version 2 file without any.
/// - 3: the feature timeline. Older files read as having none.
/// - 4: sketches on faces of the part.
/// - 5: hole features.
/// - 6: chamfer and fillet features.
/// - 7: expressions driving dimensions and extrude distances.
/// - 8: pattern features.
/// - 9: CAM definitions: stock, tools, setups, operations (decision 0014).
/// - 10: facing operations.
/// - 11: contour operations.
/// - 12: pocket operations.
/// - 13: chamfer milling operations.
/// - 14: finishing allowances on pockets.
/// - 15: setups turned about Y, and B table angles (decision 0015).
/// - 16: measured tool assemblies (decision 0016).
/// - 17: imported solids, their files under `design/imports/` (decision 0017).
/// - 18: imported IGES files.
/// - 19: STL reference meshes (decision 0018).
/// - 20: setups on the part's sides (decision 0019).
/// - 21: a vise per setup (decision 0020).
/// - 22: revolve features (decision 0021).
pub const SCHEMA_VERSION: u32 = 22;

/// The file extension for project files.
pub const EXTENSION: &str = "cndt";

const MANIFEST: &str = "manifest.json";
const DOCUMENT: &str = "design/document.json";
const EXTENSIONS: &str = "extensions/";
/// Imported files, one entry per import feature: `design/imports/<id>.step`
/// or `.igs`.
const IMPORTS: &str = "design/imports/";

/// The largest entry this build will read, after decompression.
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
/// The most entries a project may hold.
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    schema_version: u32,
    app_version: String,
    units: String,
}

/// A project as stored on disk: the document plus extension data.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The design document.
    pub document: Document,
    /// Module data keyed by entry name under `extensions/`. Preserved
    /// byte-for-byte on save, even when no installed module understands it.
    extensions: BTreeMap<String, Vec<u8>>,
}

impl Project {
    /// A project holding `document` and no extension data.
    #[must_use]
    pub const fn new(document: Document) -> Self {
        Self {
            document,
            extensions: BTreeMap::new(),
        }
    }

    /// Extension entries, keyed by their path under `extensions/`.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.extensions
    }

    /// Adds or replaces an extension entry.
    ///
    /// # Errors
    ///
    /// Returns [`FormatError::BadEntryName`] if `name` is not a safe relative
    /// path.
    pub fn set_extension(&mut self, name: &str, data: Vec<u8>) -> Result<(), FormatError> {
        check_entry_name(name)?;
        self.extensions.insert(name.to_owned(), data);
        Ok(())
    }
}

/// Why a project could not be read or written.
#[derive(Debug)]
pub enum FormatError {
    /// The file system refused.
    Io(io::Error),
    /// The file is not a CNDT project at all.
    NotAProject(String),
    /// The file was written by a newer CNDT.
    NewerVersion {
        /// The schema version in the file.
        found: u32,
        /// The newest version this build reads.
        supported: u32,
    },
    /// The file is a CNDT project but is damaged or malformed.
    Corrupt(String),
    /// The project's content breaks a model rule.
    InvalidContent(ModelError),
    /// An entry name is not a safe relative path.
    BadEntryName(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::NotAProject(reason) => write!(f, "not a CNDT project: {reason}"),
            Self::NewerVersion { found, supported } => write!(
                f,
                "this project was saved by a newer CNDT (file version {found}, \
                 this build reads up to {supported}); update CNDT to open it"
            ),
            Self::Corrupt(reason) => write!(f, "the project file is damaged: {reason}"),
            Self::InvalidContent(error) => write!(f, "the project content is invalid: {error}"),
            Self::BadEntryName(name) => write!(f, "unsafe entry name {name:?}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<io::Error> for FormatError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Writes a project to any seekable writer.
///
/// # Errors
///
/// Returns an error if writing fails.
pub fn write<W: Write + Seek>(project: &Project, writer: W) -> Result<(), FormatError> {
    let manifest = Manifest {
        format: String::from(FORMAT_NAME),
        schema_version: SCHEMA_VERSION,
        app_version: String::from(env!("CARGO_PKG_VERSION")),
        units: String::from("mm"),
    };

    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    entries.insert(String::from(MANIFEST), to_json(&manifest)?);
    entries.insert(String::from(DOCUMENT), to_json(&project.document)?);
    for entry in project.document.features() {
        if let cndt_model::Feature::Import { data, .. }
        | cndt_model::Feature::Reference { data, .. } = &entry.feature
        {
            entries.insert(
                import_entry(&entry.feature, entry.id),
                data.0.as_ref().clone(),
            );
        }
    }
    for (name, data) in &project.extensions {
        check_entry_name(name)?;
        entries.insert(format!("{EXTENSIONS}{name}"), data.clone());
    }

    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let mut zip = ZipWriter::new(writer);
    for (name, data) in &entries {
        zip.start_file(name.as_str(), options).map_err(zip_error)?;
        zip.write_all(data)?;
    }
    zip.finish().map_err(zip_error)?;
    Ok(())
}

/// Reads a project from any seekable reader.
///
/// # Errors
///
/// Returns an error if the data is not a CNDT project, was written by a
/// newer version, is damaged, or holds an invalid document.
pub fn read<R: Read + Seek>(reader: R) -> Result<Project, FormatError> {
    let mut zip = ZipArchive::new(reader)
        .map_err(|error| FormatError::NotAProject(format!("not a ZIP container ({error})")))?;
    if zip.len() > MAX_ENTRIES {
        return Err(FormatError::Corrupt(format!(
            "{} entries is more than the limit of {MAX_ENTRIES}",
            zip.len()
        )));
    }

    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(zip_error)?;
        let name = file.name().map_err(zip_error)?.into_owned();
        if file.is_dir() {
            continue;
        }
        if file.size() > MAX_ENTRY_BYTES {
            return Err(FormatError::Corrupt(format!("{name} is too large")));
        }
        let mut data = Vec::new();
        (&mut file)
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > MAX_ENTRY_BYTES {
            return Err(FormatError::Corrupt(format!("{name} is too large")));
        }
        entries.insert(name, data);
    }

    let manifest_bytes = entries
        .remove(MANIFEST)
        .ok_or_else(|| FormatError::NotAProject(String::from("it has no manifest.json")))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| FormatError::NotAProject(format!("unreadable manifest ({error})")))?;
    if manifest.format != FORMAT_NAME {
        return Err(FormatError::NotAProject(format!(
            "the manifest names format {:?}",
            manifest.format
        )));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(FormatError::NewerVersion {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    if manifest.schema_version == 0 {
        return Err(FormatError::Corrupt(String::from("schema version 0")));
    }
    if manifest.units != "mm" {
        return Err(FormatError::Corrupt(format!(
            "unsupported units {:?}",
            manifest.units
        )));
    }

    let document_bytes = entries
        .remove(DOCUMENT)
        .ok_or_else(|| FormatError::Corrupt(format!("{DOCUMENT} is missing")))?;
    let mut document: Document = serde_json::from_slice(&document_bytes)
        .map_err(|error| FormatError::Corrupt(format!("{DOCUMENT}: {error}")))?;
    // Each import's file is its own entry.
    let imports: Vec<(cndt_model::FeatureId, String)> = document
        .features()
        .iter()
        .filter(|e| {
            matches!(
                e.feature,
                cndt_model::Feature::Import { .. } | cndt_model::Feature::Reference { .. }
            )
        })
        .map(|e| (e.id, import_entry(&e.feature, e.id)))
        .collect();
    for (id, name) in imports {
        let data = entries
            .remove(&name)
            .ok_or_else(|| FormatError::Corrupt(format!("{name} is missing")))?;
        document
            .load_import_data(id, cndt_model::ImportData(std::sync::Arc::new(data)))
            .map_err(FormatError::InvalidContent)?;
    }
    document.validate().map_err(FormatError::InvalidContent)?;
    // Version 21 knew nothing of revolves.
    if manifest.schema_version < 22
        && document
            .features()
            .iter()
            .any(|e| matches!(e.feature, cndt_model::Feature::Revolve { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain revolve features",
            manifest.schema_version
        )));
    }
    // Version 20 knew nothing of vises.
    if manifest.schema_version < 21 && document.cam().setups.iter().any(|s| s.setup.vise.is_some())
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain vises",
            manifest.schema_version
        )));
    }
    // Version 19 knew nothing of side setups.
    if manifest.schema_version < 20
        && document
            .cam()
            .setups
            .iter()
            .any(|s| s.setup.side.is_lateral())
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain setups on the part's sides",
            manifest.schema_version
        )));
    }
    // Version 18 knew nothing of reference meshes.
    if manifest.schema_version < 19
        && document
            .features()
            .iter()
            .any(|e| matches!(e.feature, cndt_model::Feature::Reference { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain reference meshes",
            manifest.schema_version
        )));
    }
    // Version 17 knew nothing of IGES imports.
    if manifest.schema_version < 18
        && document.features().iter().any(|e| {
            matches!(
                e.feature,
                cndt_model::Feature::Import {
                    format: cndt_model::ImportFormat::Iges,
                    ..
                }
            )
        })
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain imported IGES files",
            manifest.schema_version
        )));
    }
    // Version 16 knew nothing of imported solids.
    if manifest.schema_version < 17
        && document
            .features()
            .iter()
            .any(|e| matches!(e.feature, cndt_model::Feature::Import { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain imported solids",
            manifest.schema_version
        )));
    }
    // Migrations. Version 1 knew nothing of sketches.
    if manifest.schema_version < 2 && !document.sketches().is_empty() {
        return Err(FormatError::Corrupt(String::from(
            "a version 1 file cannot contain sketches",
        )));
    }
    // Version 15 knew nothing of tool assemblies.
    if manifest.schema_version < 16 && document.cam().tools.values().any(|t| t.assembly.is_some()) {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain tool assemblies",
            manifest.schema_version
        )));
    }
    // Version 14 knew nothing of turning about Y or of B angles.
    if manifest.schema_version < 15
        && document
            .cam()
            .setups
            .iter()
            .any(|s| s.setup.flip_about != cndt_model::cam::FlipAxis::X || s.setup.index.is_some())
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain B angles or setups turned about Y",
            manifest.schema_version
        )));
    }
    // Version 13 knew nothing of pocket finishing allowances.
    if manifest.schema_version < 14
        && document
            .cam()
            .setups
            .iter()
            .flat_map(|s| &s.setup.operations)
            .any(|o| {
                matches!(
                    o.operation,
                    cndt_model::cam::OperationDef::Pocket {
                        finish: Some(_),
                        ..
                    }
                )
            })
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain pocket finishing allowances",
            manifest.schema_version
        )));
    }
    // Version 12 knew nothing of chamfer milling.
    if manifest.schema_version < 13
        && document
            .cam()
            .setups
            .iter()
            .flat_map(|s| &s.setup.operations)
            .any(|o| matches!(o.operation, cndt_model::cam::OperationDef::Chamfer { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain chamfer milling operations",
            manifest.schema_version
        )));
    }
    // Version 11 knew nothing of pockets.
    if manifest.schema_version < 12
        && document
            .cam()
            .setups
            .iter()
            .flat_map(|s| &s.setup.operations)
            .any(|o| matches!(o.operation, cndt_model::cam::OperationDef::Pocket { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain pocket operations",
            manifest.schema_version
        )));
    }
    // Version 10 knew nothing of contours.
    if manifest.schema_version < 11
        && document
            .cam()
            .setups
            .iter()
            .flat_map(|s| &s.setup.operations)
            .any(|o| matches!(o.operation, cndt_model::cam::OperationDef::Contour { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain contour operations",
            manifest.schema_version
        )));
    }
    // Version 9 knew nothing of facing.
    if manifest.schema_version < 10
        && document
            .cam()
            .setups
            .iter()
            .flat_map(|s| &s.setup.operations)
            .any(|o| matches!(o.operation, cndt_model::cam::OperationDef::Face { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain facing operations",
            manifest.schema_version
        )));
    }
    // Version 8 knew nothing of CAM definitions.
    if manifest.schema_version < 9 && !document.cam().is_empty() {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain CAM setups or tools",
            manifest.schema_version
        )));
    }
    // Version 7 knew nothing of patterns.
    if manifest.schema_version < 8
        && document
            .features()
            .iter()
            .any(|e| matches!(e.feature, cndt_model::Feature::Pattern { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain patterns",
            manifest.schema_version
        )));
    }
    // Version 6 knew nothing of expressions.
    let has_expressions = document
        .sketches()
        .values()
        .any(|r| !r.expressions.is_empty())
        || document.features().iter().any(|e| {
            matches!(
                &e.feature,
                cndt_model::Feature::Extrude {
                    distance_expression: Some(_),
                    ..
                }
            )
        });
    if manifest.schema_version < 7 && has_expressions {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain expressions",
            manifest.schema_version
        )));
    }
    // Version 5 knew nothing of chamfers and fillets.
    if manifest.schema_version < 6
        && document.features().iter().any(|entry| {
            matches!(
                entry.feature,
                cndt_model::Feature::Chamfer { .. } | cndt_model::Feature::Fillet { .. }
            )
        })
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain chamfers or fillets",
            manifest.schema_version
        )));
    }
    // Version 4 knew nothing of holes.
    if manifest.schema_version < 5
        && document
            .features()
            .iter()
            .any(|entry| matches!(entry.feature, cndt_model::Feature::Hole { .. }))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain hole features",
            manifest.schema_version
        )));
    }
    // Version 3 knew nothing of sketches on faces.
    if manifest.schema_version < 4
        && document
            .sketches()
            .values()
            .any(|record| matches!(record.plane, cndt_model::Plane::Face(_)))
    {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain sketches on faces",
            manifest.schema_version
        )));
    }
    // Version 2 knew nothing of features.
    if manifest.schema_version < 3 && !document.features().is_empty() {
        return Err(FormatError::Corrupt(format!(
            "a version {} file cannot contain features",
            manifest.schema_version
        )));
    }

    let mut project = Project::new(document);
    for (name, data) in entries {
        let Some(extension) = name.strip_prefix(EXTENSIONS) else {
            // Within one schema version every entry is known. Refusing an
            // unknown one is safer than silently dropping it on the next save.
            return Err(FormatError::Corrupt(format!("unexpected entry {name}")));
        };
        project.set_extension(extension, data)?;
    }
    Ok(project)
}

/// Saves a project to a path, atomically: the data is written to a
/// temporary file next to the target, flushed to disk, then renamed over it.
///
/// # Errors
///
/// Returns an error if any step fails. The target is then left as it was.
pub fn save(project: &Project, path: &Path) -> Result<(), FormatError> {
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = File::create(&temporary)?;
        write(project, &mut file)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        // Best effort: the temporary file is useless once saving failed.
        let _ignored = fs::remove_file(&temporary);
    }
    result
}

/// Opens a project file.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read as a project.
pub fn open(path: &Path) -> Result<Project, FormatError> {
    read(File::open(path)?)
}

/// Where an import feature's file is stored.
fn import_entry(feature: &cndt_model::Feature, id: cndt_model::FeatureId) -> String {
    let extension = match feature {
        cndt_model::Feature::Import { format, .. } => format.extension(),
        cndt_model::Feature::Reference { .. } => "stl",
        _ => "step",
    };
    format!("{IMPORTS}{}.{extension}", id.0)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".saving");
    path.with_file_name(name)
}

fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>, FormatError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| FormatError::Corrupt(format!("cannot encode JSON: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn zip_error(error: zip::result::ZipError) -> FormatError {
    match error {
        zip::result::ZipError::Io(error) => FormatError::Io(error),
        other => FormatError::Corrupt(other.to_string()),
    }
}

/// A safe entry name: relative, `/`-separated, no empty, `.` or `..` parts,
/// no backslashes or drive letters.
fn check_entry_name(name: &str) -> Result<(), FormatError> {
    let safe = !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains(':')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if safe {
        Ok(())
    } else {
        Err(FormatError::BadEntryName(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cndt_model::{Parameter, Unit};
    use std::io::Cursor;

    fn sample() -> Project {
        let mut document = Document::new("Bracket-A12");
        document
            .set_parameter("thickness", Parameter::new(12.0, Unit::Millimetre))
            .unwrap();
        document
            .set_parameter("holes", Parameter::new(4.0, Unit::Unitless))
            .unwrap();
        Project::new(document)
    }

    fn to_bytes(project: &Project) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        write(project, &mut buffer).unwrap();
        buffer.into_inner()
    }

    /// Builds a ZIP with arbitrary entries, for testing bad input.
    fn raw_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn manifest(version: u32) -> String {
        format!(
            r#"{{"format":"cndt-project","schema_version":{version},"app_version":"9.9.9","units":"mm"}}"#
        )
    }

    #[test]
    fn round_trip() {
        let project = sample();
        let back = read(Cursor::new(to_bytes(&project))).unwrap();
        assert_eq!(back, project);
    }

    #[test]
    fn the_same_project_gives_the_same_bytes() {
        assert_eq!(to_bytes(&sample()), to_bytes(&sample()));
    }

    #[test]
    fn unknown_extension_data_survives_a_round_trip() {
        let mut project = sample();
        project
            .set_extension("future-module/data.bin", vec![0, 1, 2, 255])
            .unwrap();
        let back = read(Cursor::new(to_bytes(&project))).unwrap();
        assert_eq!(
            back.extensions().get("future-module/data.bin"),
            Some(&vec![0, 1, 2, 255])
        );
    }

    #[test]
    fn sketches_round_trip_and_version_1_files_still_open() {
        use cndt_model::Plane;
        use cndt_model::sketch::{Constraint, Point2, SolveOptions};
        let mut document = Document::new("Test block");
        let id = document.add_sketch("", Plane::Xy).unwrap();
        document
            .edit_sketch(id, |s| {
                let a = s.add_point(Point2::new(0.0, 0.0))?;
                let b = s.add_point(Point2::new(96.0, 0.5))?;
                let line = s.add_line(a, b)?;
                s.add_constraint(Constraint::Horizontal { line }, &SolveOptions::default())
            })
            .unwrap();
        document
            .add_feature(cndt_model::Feature::Extrude {
                name: String::from("Extrude1"),
                sketch: id,
                profiles: vec![cndt_model::sketch::ProfileKey(vec![
                    cndt_model::sketch::EntityId(2),
                ])],
                distance: 20.0,
                distance_expression: None,
                operation: cndt_model::Operation::Join,
            })
            .unwrap();
        let project = Project::new(document);
        let bytes = to_bytes(&project);
        assert_eq!(read(Cursor::new(bytes)).unwrap(), project);
        let v2 = raw_zip(&[
            (MANIFEST, &manifest(2)),
            (
                DOCUMENT,
                r#"{"name":"Old","sketches":{"0":{"name":"S","plane":"xy","sketch":{"entities":{},"constraints":{},"next_entity":0,"next_constraint":0}}},"next_sketch":1,"features":[{"id":0,"feature":{"type":"extrude","name":"E","sketch":0,"profiles":[[1]],"distance":1.0,"operation":"join"}}],"next_feature":1}"#,
            ),
        ]);
        assert!(matches!(
            read(Cursor::new(v2)),
            Err(FormatError::Corrupt(_))
        ));

        let v1 = raw_zip(&[
            (MANIFEST, &manifest(1)),
            (DOCUMENT, r#"{"name":"Old","parameters":{}}"#),
        ]);
        assert_eq!(read(Cursor::new(v1)).unwrap().document.name(), "Old");
        let forged = raw_zip(&[
            (MANIFEST, &manifest(1)),
            (
                DOCUMENT,
                r#"{"name":"Old","sketches":{"0":{"name":"S","plane":"xy","sketch":{"entities":{},"constraints":{},"next_entity":0,"next_constraint":0}}},"next_sketch":1}"#,
            ),
        ]);
        assert!(matches!(
            read(Cursor::new(forged)),
            Err(FormatError::Corrupt(_))
        ));
    }

    fn imported(content: &str) -> Document {
        let mut document = Document::new("Imported");
        let data = cndt_model::ImportData(std::sync::Arc::new(content.as_bytes().to_vec()));
        let (bytes, hash) = data.fingerprint();
        document
            .add_feature(cndt_model::Feature::Import {
                name: String::from("Import1"),
                source: String::from("bracket.step"),
                format: cndt_model::ImportFormat::Step,
                operation: cndt_model::Operation::Join,
                bytes,
                hash,
                data,
            })
            .unwrap();
        document
    }

    #[test]
    fn imported_files_are_stored_as_their_own_entries() {
        let content = "ISO-10303-21;\nHEADER;\nENDSEC;\nEND-ISO-10303-21;\n";
        let project = Project::new(imported(content));
        let bytes = to_bytes(&project);
        let back = read(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(back, project);
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut stored = String::new();
        zip.by_name("design/imports/0.step")
            .unwrap()
            .read_to_string(&mut stored)
            .unwrap();
        assert_eq!(stored, content);
        // The JSON holds the fingerprint, never the content.
        let mut json = String::new();
        zip.by_name(DOCUMENT)
            .unwrap()
            .read_to_string(&mut json)
            .unwrap();
        assert!(
            json.contains("bracket.step") && !json.contains("ENDSEC"),
            "{json}"
        );
    }

    #[test]
    fn a_missing_or_changed_import_is_refused() {
        let json = serde_json::to_string(&imported("ISO-10303-21; one")).unwrap();
        let missing = raw_zip(&[(MANIFEST, &manifest(17)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(missing)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("design/imports/0.step is missing"),
            "{error}"
        );
        let changed = raw_zip(&[
            (MANIFEST, &manifest(17)),
            (DOCUMENT, &json),
            ("design/imports/0.step", "ISO-10303-21; two"),
        ]);
        let error = read(Cursor::new(changed)).unwrap_err();
        assert!(error.to_string().contains("does not match"), "{error}");
        let stray = raw_zip(&[
            (MANIFEST, &manifest(17)),
            (DOCUMENT, &json),
            ("design/imports/0.step", "ISO-10303-21; one"),
            ("design/imports/5.step", "ISO-10303-21; five"),
        ]);
        let error = read(Cursor::new(stray)).unwrap_err();
        assert!(error.to_string().contains("unexpected entry"), "{error}");
    }

    #[test]
    fn iges_imports_need_version_18_and_keep_their_extension() {
        let mut document = imported("ISO-10303-21; one");
        let data = cndt_model::ImportData(std::sync::Arc::new(b"iges".to_vec()));
        let (bytes, hash) = data.fingerprint();
        document
            .add_feature(cndt_model::Feature::Import {
                name: String::from("Import2"),
                source: String::from("plate.igs"),
                format: cndt_model::ImportFormat::Iges,
                operation: cndt_model::Operation::Cut,
                bytes,
                hash,
                data,
            })
            .unwrap();
        let project = Project::new(document.clone());
        let bytes = to_bytes(&project);
        assert_eq!(read(Cursor::new(bytes.clone())).unwrap(), project);
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(zip.by_name("design/imports/1.igs").is_ok());
        let json = serde_json::to_string(&document).unwrap();
        let v17 = raw_zip(&[
            (MANIFEST, &manifest(17)),
            (DOCUMENT, &json),
            ("design/imports/0.step", "ISO-10303-21; one"),
            ("design/imports/1.igs", "iges"),
        ]);
        let error = read(Cursor::new(v17)).unwrap_err();
        assert!(error.to_string().contains("IGES"), "{error}");
    }

    #[test]
    fn reference_meshes_are_stored_as_stl_and_need_version_19() {
        let mut document = Document::new("Mesh");
        let data = cndt_model::ImportData(std::sync::Arc::new(vec![0, 1, 2, 255]));
        let (bytes, hash) = data.fingerprint();
        document
            .add_feature(cndt_model::Feature::Reference {
                name: String::from("Mesh1"),
                source: String::from("scan.stl"),
                bytes,
                hash,
                data,
            })
            .unwrap();
        let project = Project::new(document.clone());
        let bytes = to_bytes(&project);
        assert_eq!(read(Cursor::new(bytes.clone())).unwrap(), project);
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut stored = Vec::new();
        zip.by_name("design/imports/0.stl")
            .unwrap()
            .read_to_end(&mut stored)
            .unwrap();
        assert_eq!(stored, vec![0, 1, 2, 255], "binary content kept exactly");
        let json = serde_json::to_string(&document).unwrap();
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in [
            (MANIFEST, manifest(18).into_bytes()),
            (DOCUMENT, json.into_bytes()),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&content).unwrap();
        }
        zip.start_file("design/imports/0.stl", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&[0, 1, 2, 255]).unwrap();
        let v18 = zip.finish().unwrap().into_inner();
        let error = read(Cursor::new(v18)).unwrap_err();
        assert!(error.to_string().contains("reference meshes"), "{error}");
    }

    #[test]
    fn imports_need_version_17() {
        let json = serde_json::to_string(&imported("ISO-10303-21; one")).unwrap();
        let v16 = raw_zip(&[
            (MANIFEST, &manifest(16)),
            (DOCUMENT, &json),
            ("design/imports/0.step", "ISO-10303-21; one"),
        ]);
        let error = read(Cursor::new(v16)).unwrap_err();
        assert!(error.to_string().contains("imported solids"), "{error}");
    }

    #[test]
    fn tool_assemblies_need_version_16() {
        use cndt_model::cam::{Assembly, CamEdit, Tool, ToolKind};
        let mut document = Document::new("Tools");
        document
            .edit_cam(CamEdit::SetTool {
                number: 3,
                tool: Tool {
                    kind: ToolKind::EndMill,
                    diameter: 10.0,
                    rpm: 8000.0,
                    feed: 1200.0,
                    description: String::new(),
                    assembly: Some(Assembly {
                        flute_length: 22.0,
                        stickout: 40.0,
                        shank_diameter: 10.0,
                        holder_diameter: 50.0,
                        holder_length: 60.0,
                    }),
                },
            })
            .unwrap();
        let project = Project::new(document.clone());
        assert_eq!(read(Cursor::new(to_bytes(&project))).unwrap(), project);
        let json = serde_json::to_string(&document).unwrap();
        let v15 = raw_zip(&[(MANIFEST, &manifest(15)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(v15)).unwrap_err();
        assert!(error.to_string().contains("tool assemblies"), "{error}");
    }

    #[test]
    fn revolves_need_version_22() {
        use cndt_model::sketch::Point2;
        use cndt_model::{Feature, Operation, Plane, RevolveAxis, default_profiles};
        let mut document = Document::new("Shaft");
        let sketch = document.add_sketch("Half", Plane::Xy).unwrap();
        document
            .edit_sketch(sketch, |s| {
                let p: Vec<_> = [(0.0, 0.0), (5.0, 0.0), (5.0, 10.0), (0.0, 10.0)]
                    .iter()
                    .map(|&(x, y)| s.add_point(Point2::new(x, y)))
                    .collect::<Result<_, _>>()?;
                for i in 0..4 {
                    s.add_line(p[i], p[(i + 1) % 4])?;
                }
                Ok(())
            })
            .unwrap();
        let profiles = default_profiles(&document.sketch(sketch).unwrap().sketch);
        document
            .add_feature(Feature::Revolve {
                name: String::from("Revolve1"),
                sketch,
                profiles,
                axis: RevolveAxis::SketchY,
                angle: 180.0,
                angle_expression: None,
                operation: Operation::Join,
            })
            .unwrap();
        let project = Project::new(document.clone());
        assert_eq!(read(Cursor::new(to_bytes(&project))).unwrap(), project);
        let json = serde_json::to_string(&document).unwrap();
        let v21 = raw_zip(&[(MANIFEST, &manifest(21)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(v21)).unwrap_err();
        assert!(error.to_string().contains("revolve"), "{error}");
    }

    #[test]
    fn vises_need_version_21() {
        use cndt_model::cam::{CamEdit, FlipAxis, JawAxis, Side, Vise};
        let mut document = Document::new("Vise");
        let id = document
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Top,
                work_offset: 1,
                flip_about: FlipAxis::X,
                index: None,
            })
            .unwrap()
            .unwrap();
        document
            .edit_cam(CamEdit::SetVise {
                setup: id,
                vise: Some(Vise {
                    grip: 8.0,
                    along: JawAxis::Y,
                    jaw: 20.0,
                }),
            })
            .unwrap();
        let project = Project::new(document.clone());
        assert_eq!(read(Cursor::new(to_bytes(&project))).unwrap(), project);
        let json = serde_json::to_string(&document).unwrap();
        let v20 = raw_zip(&[(MANIFEST, &manifest(20)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(v20)).unwrap_err();
        assert!(error.to_string().contains("vises"), "{error}");
    }

    #[test]
    fn side_setups_need_version_20() {
        use cndt_model::cam::{CamEdit, FlipAxis, Side};
        let mut document = Document::new("Sides");
        document
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Right,
                work_offset: 2,
                flip_about: FlipAxis::X,
                index: Some(90.0),
            })
            .unwrap();
        assert_eq!(document.cam().setups[0].setup.name, "Setup 1 (right)");
        let project = Project::new(document.clone());
        assert_eq!(read(Cursor::new(to_bytes(&project))).unwrap(), project);
        let json = serde_json::to_string(&document).unwrap();
        assert!(json.contains("\"side\":\"right\""), "{json}");
        let v19 = raw_zip(&[(MANIFEST, &manifest(19)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(v19)).unwrap_err();
        assert!(error.to_string().contains("sides"), "{error}");
    }

    #[test]
    fn b_angles_need_version_15() {
        use cndt_model::cam::{CamEdit, FlipAxis, Side};
        let mut document = Document::new("Indexed");
        document
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Bottom,
                work_offset: 2,
                flip_about: FlipAxis::Y,
                index: Some(180.0),
            })
            .unwrap();
        let project = Project::new(document.clone());
        assert_eq!(read(Cursor::new(to_bytes(&project))).unwrap(), project);
        let json = serde_json::to_string(&document).unwrap();
        let v14 = raw_zip(&[(MANIFEST, &manifest(14)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(v14)).unwrap_err();
        assert!(error.to_string().contains("B angles"), "{error}");
    }

    #[test]
    fn a_pocket_finish_needs_version_14() {
        use cndt_model::cam::{CamEdit, OperationDef, Side};
        let mut document = Document::new("Finish");
        document
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Top,
                work_offset: 1,
                flip_about: cndt_model::cam::FlipAxis::X,
                index: None,
            })
            .unwrap();
        let setup = document.cam().setups[0].id;
        document
            .edit_cam(CamEdit::AddOperation {
                setup,
                operation: OperationDef::Pocket {
                    floor: String::from("f1/end/e0"),
                    tool: None,
                    finish: Some(0.2),
                },
            })
            .unwrap();
        let project = Project::new(document.clone());
        assert_eq!(read(Cursor::new(to_bytes(&project))).unwrap(), project);
        let json = serde_json::to_string(&document).unwrap();
        let v13 = raw_zip(&[(MANIFEST, &manifest(13)), (DOCUMENT, &json)]);
        let error = read(Cursor::new(v13)).unwrap_err();
        assert!(error.to_string().contains("finishing"), "{error}");
    }

    #[test]
    fn a_newer_file_is_refused_with_a_clear_message() {
        let bytes = raw_zip(&[(MANIFEST, &manifest(SCHEMA_VERSION + 1))]);
        let error = read(Cursor::new(bytes)).unwrap_err();
        assert!(
            matches!(error, FormatError::NewerVersion { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("update CNDT"), "{error}");
    }

    #[test]
    fn other_files_are_not_projects() {
        let not_zip = read(Cursor::new(b"hello".to_vec())).unwrap_err();
        assert!(matches!(not_zip, FormatError::NotAProject(_)));
        let wrong_format = raw_zip(&[(
            MANIFEST,
            r#"{"format":"other","schema_version":1,"app_version":"1","units":"mm"}"#,
        )]);
        assert!(matches!(
            read(Cursor::new(wrong_format)).unwrap_err(),
            FormatError::NotAProject(_)
        ));
    }

    #[test]
    fn damaged_projects_are_refused() {
        let cases = [
            raw_zip(&[(MANIFEST, &manifest(1))]),
            raw_zip(&[(MANIFEST, &manifest(1)), (DOCUMENT, "{not json")]),
            raw_zip(&[
                (MANIFEST, &manifest(1)),
                (DOCUMENT, r#"{"name":"x"}"#),
                ("stray.txt", "?"),
            ]),
        ];
        for bytes in cases {
            let error = read(Cursor::new(bytes)).unwrap_err();
            assert!(matches!(error, FormatError::Corrupt(_)), "{error:?}");
        }
    }

    #[test]
    fn an_invalid_document_is_refused() {
        let bytes = raw_zip(&[
            (MANIFEST, &manifest(1)),
            (
                DOCUMENT,
                r#"{"name":"x","parameters":{"bad name":{"value":1.0,"unit":"millimetre"}}}"#,
            ),
        ]);
        let error = read(Cursor::new(bytes)).unwrap_err();
        assert!(matches!(error, FormatError::InvalidContent(_)), "{error:?}");
    }

    #[test]
    fn unsafe_extension_names_are_refused() {
        let mut project = sample();
        for bad in ["../escape", "/abs", "a//b", "a\\b", "C:x", "", "a/./b"] {
            assert!(project.set_extension(bad, vec![]).is_err(), "{bad:?}");
        }
        let bytes = raw_zip(&[
            (MANIFEST, &manifest(1)),
            (DOCUMENT, r#"{"name":"x"}"#),
            ("extensions/../../evil", "x"),
        ]);
        assert!(matches!(
            read(Cursor::new(bytes)).unwrap_err(),
            FormatError::BadEntryName(_)
        ));
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("cndt-format-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.cndt");
        save(&sample(), &path).unwrap();
        save(&sample(), &path).unwrap();
        assert_eq!(open(&path).unwrap(), sample());
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("part.cndt")]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
