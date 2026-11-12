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
pub const SCHEMA_VERSION: u32 = 2;

/// The file extension for project files.
pub const EXTENSION: &str = "cndt";

const MANIFEST: &str = "manifest.json";
const DOCUMENT: &str = "design/document.json";
const EXTENSIONS: &str = "extensions/";

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
    let document: Document = serde_json::from_slice(&document_bytes)
        .map_err(|error| FormatError::Corrupt(format!("{DOCUMENT}: {error}")))?;
    document.validate().map_err(FormatError::InvalidContent)?;
    // Migrations. Version 1 knew nothing of sketches.
    if manifest.schema_version < 2 && !document.sketches().is_empty() {
        return Err(FormatError::Corrupt(String::from(
            "a version 1 file cannot contain sketches",
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
        let project = Project::new(document);
        let bytes = to_bytes(&project);
        assert_eq!(read(Cursor::new(bytes)).unwrap(), project);

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
