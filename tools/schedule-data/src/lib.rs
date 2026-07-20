use std::{collections::HashSet, fs, path::Path};

use serde::Deserialize;
use thiserror::Error;

mod compile;
mod diff;

pub use compile::{CompileOutput, CompileReport, compile_snapshot_batch};
pub use diff::{
    FIXED_KEIFRAME_COMMIT, KeiframeConfigDiff, KeiframeDiffError, KeiframeDiffReport,
    KeiframeTimeRecord, compare_keiframe_times, diff_keiframe,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotBatchStats {
    pub map_count: usize,
    pub table_count: usize,
    pub row_count: usize,
    pub merged_cell_count: usize,
}

pub fn validate_snapshot_batch(batch_dir: &Path) -> Result<SnapshotBatchStats, SnapshotError> {
    let manifest_path = batch_dir.join("manifest.json");
    let manifest: Manifest = read_json(&manifest_path)?;
    if manifest.schema_version != 1 {
        return Err(SnapshotError::UnsupportedSchemaVersion {
            path: manifest_path,
            version: manifest.schema_version,
        });
    }
    if manifest.map_count != manifest.maps.len() {
        return Err(SnapshotError::MapCountMismatch {
            declared: manifest.map_count,
            actual: manifest.maps.len(),
        });
    }

    let mut listed_files = HashSet::new();
    let mut table_count = 0;
    let mut row_count = 0;
    let mut merged_cell_count = 0;

    for entry in &manifest.maps {
        if !listed_files.insert(entry.file.as_str()) {
            return Err(SnapshotError::DuplicateSnapshot(entry.file.clone()));
        }

        let snapshot_path = batch_dir.join(&entry.file);
        let snapshot: Snapshot = read_json(&snapshot_path)?;
        if snapshot.schema_version != 1 {
            return Err(SnapshotError::UnsupportedSchemaVersion {
                path: snapshot_path,
                version: snapshot.schema_version,
            });
        }
        if snapshot.index_name != entry.name || snapshot.source_url != entry.source_url {
            return Err(SnapshotError::ManifestIdentityMismatch(entry.file.clone()));
        }
        if snapshot.tables.len() != entry.table_count {
            return Err(SnapshotError::TableCountMismatch {
                map: entry.name.clone(),
                declared: entry.table_count,
                actual: snapshot.tables.len(),
            });
        }

        let snapshot_rows = snapshot.tables.iter().map(|table| table.rows.len()).sum();
        if snapshot_rows != entry.row_count {
            return Err(SnapshotError::RowCountMismatch {
                map: entry.name.clone(),
                declared: entry.row_count,
                actual: snapshot_rows,
            });
        }

        for (expected_index, table) in snapshot.tables.iter().enumerate() {
            if table.table_index != expected_index {
                return Err(SnapshotError::NonContiguousTableIndex {
                    map: entry.name.clone(),
                    expected: expected_index,
                    actual: table.table_index,
                });
            }
            for row in &table.rows {
                for cell in row {
                    if cell.row_span == 0 || cell.column_span == 0 {
                        return Err(SnapshotError::ZeroCellSpan {
                            map: entry.name.clone(),
                            table_index: table.table_index,
                        });
                    }
                    merged_cell_count += usize::from(cell.row_span > 1 || cell.column_span > 1);
                }
            }
        }

        table_count += snapshot.tables.len();
        row_count += snapshot_rows;
    }

    let actual_files = fs::read_dir(batch_dir)
        .map_err(|source| SnapshotError::Read {
            path: batch_dir.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json") && name != "manifest.json")
        .collect::<HashSet<_>>();
    let listed_files = listed_files
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();
    if actual_files != listed_files {
        return Err(SnapshotError::SnapshotSetMismatch);
    }

    Ok(SnapshotBatchStats {
        map_count: manifest.maps.len(),
        table_count,
        row_count,
        merged_cell_count,
    })
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, SnapshotError> {
    let bytes = fs::read(path).map_err(|source| SnapshotError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| SnapshotError::Json {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) schema_version: u32,
    pub(crate) map_count: usize,
    pub(crate) maps: Vec<ManifestMap>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ManifestMap {
    pub(crate) name: String,
    pub(crate) source_url: String,
    pub(crate) file: String,
    pub(crate) table_count: usize,
    pub(crate) row_count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Snapshot {
    pub(crate) schema_version: u32,
    pub(crate) index_name: String,
    pub(crate) source_url: String,
    pub(crate) tables: Vec<SnapshotTable>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SnapshotTable {
    pub(crate) table_index: usize,
    pub(crate) rows: Vec<Vec<SnapshotCell>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SnapshotCell {
    pub(crate) text: String,
    pub(crate) row_span: usize,
    pub(crate) column_span: usize,
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: std::path::PathBuf,
        source: serde_json::Error,
    },
    #[error("unsupported snapshot schema version {version} in {path}")]
    UnsupportedSchemaVersion {
        path: std::path::PathBuf,
        version: u32,
    },
    #[error("manifest declares {declared} maps but contains {actual}")]
    MapCountMismatch { declared: usize, actual: usize },
    #[error("manifest contains duplicate snapshot {0}")]
    DuplicateSnapshot(String),
    #[error("manifest identity does not match snapshot {0}")]
    ManifestIdentityMismatch(String),
    #[error("map {map} declares {declared} tables but contains {actual}")]
    TableCountMismatch {
        map: String,
        declared: usize,
        actual: usize,
    },
    #[error("map {map} declares {declared} rows but contains {actual}")]
    RowCountMismatch {
        map: String,
        declared: usize,
        actual: usize,
    },
    #[error("map {map} expected table index {expected} but found {actual}")]
    NonContiguousTableIndex {
        map: String,
        expected: usize,
        actual: usize,
    },
    #[error("map {map} table {table_index} contains a zero cell span")]
    ZeroCellSpan { map: String, table_index: usize },
    #[error("snapshot JSON file set does not match the manifest")]
    SnapshotSetMismatch,
}
