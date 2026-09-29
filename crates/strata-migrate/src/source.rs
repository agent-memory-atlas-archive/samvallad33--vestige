//! Source loading: turn `<src>` into a `vestige.portable.v1` archive.
//!
//! Path A (preferred): `<src>` is a portable-archive JSON file produced by
//! `vestige portable-export`. Decode it directly.
//!
//! Path B (direct): `<src>` is a SQLite database file (or a data directory
//! containing `vestige.db`). Open it with `vestige-core` `Storage::new` and
//! call the SAME export code `vestige portable-export` uses, so both paths
//! feed identical [`PortableArchive`] structures downstream and there is
//! exactly one row-decoding implementation.
//!
//! Path B side effects (documented, accepted): opening a live store via
//! `Storage::new` attaches SQLite WAL journaling and may create `-wal`/`-shm`
//! siblings next to the database, and idempotent schema migrations run on
//! open. The store's logical contents are never modified — migration is
//! read-only over the export snapshot.

use std::path::{Path, PathBuf};

use chrono::DateTime;
use vestige_core::storage::{PortableTable, PortableValue};
use vestige_core::{PortableArchive, PORTABLE_ARCHIVE_FORMAT, SqliteMemoryStore};

use crate::MigrationError;

/// SQLite file magic (first 16 bytes of every SQLite 3 database).
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Load `<src>` as a portable archive, auto-detecting the path shape.
pub fn load_archive(source: &Path) -> Result<PortableArchive, MigrationError> {
    let resolved = resolve_source(source)?;

    if is_sqlite_file(&resolved)? {
        // Path B: direct SQLite walk via vestige-core's own export code.
        let storage = SqliteMemoryStore::new(Some(resolved.clone()))
            .map_err(|e| MigrationError::Source(format!("open SQLite store: {e}")))?;
        let archive = storage
            .export_portable_archive()
            .map_err(|e| MigrationError::Source(format!("export SQLite store: {e}")))?;
        Ok(archive)
    } else {
        // Path A: portable archive JSON.
        let bytes = std::fs::read(&resolved)?;
        let archive: PortableArchive = parse_archive_json(&bytes)?;
        if archive.archive_format != PORTABLE_ARCHIVE_FORMAT {
            return Err(MigrationError::UnsupportedSource(format!(
                "archive format {:?} is not {:?}",
                archive.archive_format, PORTABLE_ARCHIVE_FORMAT
            )));
        }
        Ok(archive)
    }
}

fn parse_archive_json(bytes: &[u8]) -> Result<PortableArchive, MigrationError> {
    serde_json::from_slice(bytes)
        .map_err(|e| MigrationError::UnsupportedSource(format!("not a portable archive: {e}")))
}

/// Resolve a directory source to its `vestige.db`; pass files through.
fn resolve_source(source: &Path) -> Result<PathBuf, MigrationError> {
    if source.is_dir() {
        let db = source.join("vestige.db");
        if db.is_file() {
            Ok(db)
        } else {
            Err(MigrationError::UnsupportedSource(format!(
                "directory {} has no vestige.db",
                source.display()
            )))
        }
    } else if source.is_file() {
        Ok(source.to_path_buf())
    } else {
        Err(MigrationError::SourceNotFound(source.display().to_string()))
    }
}

/// Sniff the SQLite file magic without loading the file.
fn is_sqlite_file(path: &Path) -> Result<bool, MigrationError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 16];
    let read = file.read(&mut magic)?;
    Ok(read == 16 && &magic == SQLITE_MAGIC)
}

/// Find a table by exact name.
pub fn table<'a>(archive: &'a PortableArchive, name: &str) -> Option<&'a PortableTable> {
    archive.tables.iter().find(|t| t.name == name)
}

/// Value accessor for one row of a [`PortableTable`].
pub struct Row<'a> {
    table: &'a PortableTable,
    index: usize,
}

impl<'a> Row<'a> {
    /// Wrap row `index` of `table`.
    pub fn new(table: &'a PortableTable, index: usize) -> Self {
        Self { table, index }
    }

    /// Raw value of column `name`.
    pub fn get(&self, name: &str) -> Result<&'a PortableValue, MigrationError> {
        let column = self
            .table
            .columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| {
                MigrationError::Corrupt(format!("table {} has no column {}", self.table.name, name))
            })?;
        self.table
            .rows
            .get(self.index)
            .and_then(|row| row.get(column))
            .ok_or_else(|| {
                MigrationError::Corrupt(format!(
                    "table {} row {} is missing column {}",
                    self.table.name, self.index, name
                ))
            })
    }

    /// TEXT column.
    pub fn text(&self, name: &str) -> Result<&'a str, MigrationError> {
        match self.get(name)? {
            PortableValue::Text(s) => Ok(s.as_str()),
            other => Err(type_error(&self.table.name, name, "text", other)),
        }
    }

    /// INTEGER column.
    pub fn integer(&self, name: &str) -> Result<i64, MigrationError> {
        match self.get(name)? {
            PortableValue::Integer(v) => Ok(*v),
            other => Err(type_error(&self.table.name, name, "integer", other)),
        }
    }

    /// REAL column.
    pub fn real(&self, name: &str) -> Result<f64, MigrationError> {
        match self.get(name)? {
            PortableValue::Real(v) => Ok(*v),
            PortableValue::Integer(v) => Ok(*v as f64),
            other => Err(type_error(&self.table.name, name, "real", other)),
        }
    }

    /// Nullable TEXT column.
    pub fn opt_text(&self, name: &str) -> Result<Option<&'a str>, MigrationError> {
        match self.get(name)? {
            PortableValue::Null => Ok(None),
            PortableValue::Text(s) => Ok(Some(s.as_str())),
            other => Err(type_error(&self.table.name, name, "text?", other)),
        }
    }

    /// INTEGER column with SQL NULL read as the given default.
    pub fn integer_or(&self, name: &str, default: i64) -> Result<i64, MigrationError> {
        match self.get(name)? {
            PortableValue::Null => Ok(default),
            PortableValue::Integer(v) => Ok(*v),
            other => Err(type_error(&self.table.name, name, "integer?", other)),
        }
    }

    /// REAL column with SQL NULL read as the given default.
    pub fn real_or(&self, name: &str, default: f64) -> Result<f64, MigrationError> {
        match self.get(name)? {
            PortableValue::Null => Ok(default),
            PortableValue::Real(v) => Ok(*v),
            PortableValue::Integer(v) => Ok(*v as f64),
            other => Err(type_error(&self.table.name, name, "real?", other)),
        }
    }
}

fn type_error(table: &str, column: &str, wanted: &str, got: &PortableValue) -> MigrationError {
    MigrationError::Corrupt(format!(
        "table {table} column {column}: expected {wanted}, found {got:?}"
    ))
}

/// Parse a legacy RFC3339 timestamp into Unix epoch milliseconds.
///
/// Vestige writes `DateTime<Utc>::to_rfc3339()` everywhere, but be liberal
/// about the exact offset spelling (`Z` vs `+00:00` both parse with
/// `parse_from_rfc3339`).
pub fn timestamp_ms(raw: &str) -> Result<i64, MigrationError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp_millis())
        .map_err(|e| MigrationError::Corrupt(format!("timestamp {raw:?} is not RFC3339: {e}")))
}

/// Parse a legacy JSON tag array (`'[]'`, `'["a","b"]'`). Tolerant: a NULL,
/// non-array, or malformed value migrates as no tags rather than failing the
/// whole migration — tags are metadata, not structure.
pub fn parse_tags(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}
