use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

use sc2_copilot_core::{ScheduleCatalog, Trigger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const FIXED_KEIFRAME_COMMIT: &str = "192bdbce6868e597b297cf47f485ac5c79eb9baf";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeiframeTimeRecord {
    map_name: String,
    time_value: i64,
}

impl KeiframeTimeRecord {
    pub fn new(map_name: impl Into<String>, time_value: i64) -> Self {
        Self {
            map_name: map_name.into(),
            time_value,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct KeiframeDiffReport {
    pub reference_commit: &'static str,
    pub snapshot_batch: String,
    pub configs: Vec<KeiframeConfigDiff>,
}

#[derive(Debug, Serialize)]
pub struct KeiframeConfigDiff {
    pub keiframe_config: &'static str,
    pub map_id: &'static str,
    pub variant_id: Option<&'static str>,
    pub wiki_event_count: usize,
    pub keiframe_record_count: usize,
    pub matching_times: Vec<i64>,
    pub wiki_only_times: Vec<i64>,
    pub keiframe_only_times: Vec<i64>,
    pub wiki_simultaneous_times: Vec<i64>,
    pub keiframe_simultaneous_times: Vec<i64>,
    pub wiki_window_event_count: usize,
    pub wiki_subsecond_event_count: usize,
}

pub fn compare_keiframe_times(
    catalog: &ScheduleCatalog,
    records: &[KeiframeTimeRecord],
) -> KeiframeDiffReport {
    let configs = CONFIG_MAPPINGS
        .iter()
        .map(|mapping| compare_config(catalog, records, *mapping))
        .collect();

    KeiframeDiffReport {
        reference_commit: FIXED_KEIFRAME_COMMIT,
        snapshot_batch: catalog.snapshot_batch().to_owned(),
        configs,
    }
}

pub fn diff_keiframe(
    catalog: &ScheduleCatalog,
    keiframe_repo: &Path,
) -> Result<KeiframeDiffReport, KeiframeDiffError> {
    let revision = Command::new("git")
        .arg("-C")
        .arg(keiframe_repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(KeiframeDiffError::GitIo)?;
    if !revision.status.success() {
        return Err(KeiframeDiffError::GitFailed(
            String::from_utf8_lossy(&revision.stderr).trim().to_owned(),
        ));
    }
    let actual_commit = String::from_utf8(revision.stdout)?.trim().to_owned();
    if actual_commit != FIXED_KEIFRAME_COMMIT {
        return Err(KeiframeDiffError::UnexpectedCommit {
            expected: FIXED_KEIFRAME_COMMIT,
            actual: actual_commit,
        });
    }

    let database_path = keiframe_repo.join("resources/db/maps.db");
    let query = "SELECT map_name, time_value FROM map_configs ORDER BY map_name, time_value;";
    let sqlite = Command::new("sqlite3")
        .arg("-json")
        .arg(&database_path)
        .arg(query)
        .output()
        .map_err(KeiframeDiffError::SqliteIo)?;
    if !sqlite.status.success() {
        return Err(KeiframeDiffError::SqliteFailed(
            String::from_utf8_lossy(&sqlite.stderr).trim().to_owned(),
        ));
    }
    let records: Vec<KeiframeTimeRecord> = serde_json::from_slice(&sqlite.stdout)?;

    Ok(compare_keiframe_times(catalog, &records))
}

fn compare_config(
    catalog: &ScheduleCatalog,
    records: &[KeiframeTimeRecord],
    mapping: ConfigMapping,
) -> KeiframeConfigDiff {
    let mut wiki_counts = BTreeMap::new();
    let mut wiki_event_count = 0;
    let mut wiki_window_event_count = 0;
    let mut wiki_subsecond_event_count = 0;

    if let Some(schedule) = catalog.schedule_for(mapping.map_id, mapping.variant_id) {
        for event in schedule.events().iter().filter(|event| {
            event.variant_id().is_none() || event.variant_id() == mapping.variant_id
        }) {
            match event.trigger() {
                Trigger::AtGameTime { milliseconds } if milliseconds % 1_000 == 0 => {
                    *wiki_counts
                        .entry((*milliseconds / 1_000) as i64)
                        .or_insert(0) += 1;
                    wiki_event_count += 1;
                }
                Trigger::AtGameTime { .. } => {
                    wiki_event_count += 1;
                    wiki_subsecond_event_count += 1;
                }
                Trigger::AtGameTimeWindow { .. } => {
                    wiki_event_count += 1;
                    wiki_window_event_count += 1;
                }
                Trigger::AtStageElapsed { .. } | Trigger::AtStageRemaining { .. } => {}
            }
        }
    }

    let mut keiframe_counts = BTreeMap::new();
    for record in records
        .iter()
        .filter(|record| record.map_name == mapping.keiframe_config)
    {
        *keiframe_counts.entry(record.time_value).or_insert(0) += 1;
    }

    let wiki_times = wiki_counts.keys().copied().collect::<BTreeSet<_>>();
    let keiframe_times = keiframe_counts.keys().copied().collect::<BTreeSet<_>>();

    KeiframeConfigDiff {
        keiframe_config: mapping.keiframe_config,
        map_id: mapping.map_id,
        variant_id: mapping.variant_id,
        wiki_event_count,
        keiframe_record_count: keiframe_counts.values().sum(),
        matching_times: wiki_times.intersection(&keiframe_times).copied().collect(),
        wiki_only_times: wiki_times.difference(&keiframe_times).copied().collect(),
        keiframe_only_times: keiframe_times.difference(&wiki_times).copied().collect(),
        wiki_simultaneous_times: wiki_counts
            .iter()
            .filter_map(|(time, count)| (*count > 1).then_some(*time))
            .collect(),
        keiframe_simultaneous_times: keiframe_counts
            .iter()
            .filter_map(|(time, count)| (*count > 1).then_some(*time))
            .collect(),
        wiki_window_event_count,
        wiki_subsecond_event_count,
    }
}

#[derive(Debug, Clone, Copy)]
struct ConfigMapping {
    keiframe_config: &'static str,
    map_id: &'static str,
    variant_id: Option<&'static str>,
}

const CONFIG_MAPPINGS: [ConfigMapping; 18] = [
    mapping("湮灭快车", "oblivion-express", None),
    mapping("虚空撕裂-左", "void-rifts", Some("layout-a")),
    mapping("虚空撕裂-右", "void-rifts", Some("layout-b")),
    mapping("虚空降临", "void-launch", None),
    mapping("克哈裂痕", "rifts-to-korhal", None),
    mapping("往日神庙-A", "temple-of-the-past", Some("layout-a")),
    mapping("往日神庙-B", "temple-of-the-past", Some("layout-b")),
    mapping("天界封锁", "lock-and-load", None),
    mapping("升格之链", "chain-of-ascension", None),
    mapping("熔火危机", "the-vermillion-problem", None),
    mapping("机会渺茫-人虫", "mist-opportunities", Some("terran-zerg")),
    mapping("机会渺茫-神", "mist-opportunities", Some("protoss")),
    mapping("营救矿工", "miner-evacuation", None),
    mapping("亡者之夜", "dead-of-night", None),
    mapping("黑暗杀星", "scythe-of-amon", None),
    mapping("净网行动", "malwarfare", None),
    mapping("聚铁成兵", "part-and-parcel", None),
    mapping("死亡摇篮", "cradle-of-death", None),
];

const fn mapping(
    keiframe_config: &'static str,
    map_id: &'static str,
    variant_id: Option<&'static str>,
) -> ConfigMapping {
    ConfigMapping {
        keiframe_config,
        map_id,
        variant_id,
    }
}

#[derive(Debug, Error)]
pub enum KeiframeDiffError {
    #[error("cannot run git: {0}")]
    GitIo(std::io::Error),
    #[error("git failed: {0}")]
    GitFailed(String),
    #[error("expected Keiframe commit {expected}, found {actual}")]
    UnexpectedCommit {
        expected: &'static str,
        actual: String,
    },
    #[error("cannot run sqlite3: {0}")]
    SqliteIo(std::io::Error),
    #[error("sqlite3 failed: {0}")]
    SqliteFailed(String),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
