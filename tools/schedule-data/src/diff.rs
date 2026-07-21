use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

use sc2_copilot_core::{Fact, LocationSpec, ScheduleCatalog, Trigger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const FIXED_KEIFRAME_COMMIT: &str = "192bdbce6868e597b297cf47f485ac5c79eb9baf";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeiframeTimeRecord {
    map_name: String,
    time_value: i64,
    #[serde(default)]
    time_label: Option<String>,
    #[serde(default)]
    count_value: Option<i64>,
    #[serde(default)]
    event_text: Option<String>,
    #[serde(default)]
    army_text: Option<String>,
}

impl KeiframeTimeRecord {
    pub fn new(map_name: impl Into<String>, time_value: i64) -> Self {
        Self {
            map_name: map_name.into(),
            time_value,
            time_label: None,
            count_value: None,
            event_text: None,
            army_text: None,
        }
    }

    pub fn with_comparison_fields(
        map_name: impl Into<String>,
        time_label: impl Into<String>,
        time_value: i64,
        event_text: Option<&str>,
        army_text: Option<&str>,
    ) -> Self {
        Self {
            map_name: map_name.into(),
            time_value,
            time_label: Some(time_label.into()),
            count_value: None,
            event_text: event_text.map(str::to_owned),
            army_text: army_text.map(str::to_owned),
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
    pub keiframe_time_label_mismatch_count: usize,
    pub wiki_structured_fact_count: usize,
    pub wiki_multi_fact_event_count: usize,
    pub keiframe_text_attribute_record_count: usize,
    pub keiframe_compound_record_count: usize,
    pub keiframe_count_condition_record_count: usize,
    pub numeric_candidate_review_times: Vec<i64>,
    pub numeric_candidate_mismatch_times: Vec<i64>,
    pub category_review_times: Vec<i64>,
    pub simultaneous_count_mismatches: Vec<SimultaneousCountDiff>,
    pub shared_vocabulary_token_count: usize,
    pub wiki_only_vocabulary_token_count: usize,
    pub keiframe_only_vocabulary_token_count: usize,
    pub unsupported_runtime_condition: bool,
}

#[derive(Debug, Serialize)]
pub struct SimultaneousCountDiff {
    pub time_value: i64,
    pub wiki_count: usize,
    pub keiframe_count: usize,
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
    let query = "SELECT map_name, time_label, time_value, CASE WHEN typeof(count_value) = 'integer' THEN count_value END AS count_value, event_text, army_text FROM map_configs ORDER BY map_name, time_value;";
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
    let mut wiki_structured_fact_count = 0;
    let mut wiki_multi_fact_event_count = 0;
    let mut wiki_vocabulary = BTreeSet::new();
    let mut wiki_numbers = BTreeMap::<i64, BTreeSet<u64>>::new();
    let mut wiki_categories = BTreeSet::<i64>::new();

    if let Some(schedule) = catalog.schedule_for(mapping.map_id, mapping.variant_id) {
        for event in schedule.events().iter().filter(|event| {
            event.variant_id().is_none() || event.variant_id() == mapping.variant_id
        }) {
            wiki_structured_fact_count += event.facts().len();
            if event.facts().len() > 1 {
                wiki_multi_fact_event_count += 1;
            }
            collect_wiki_vocabulary(event.facts(), &mut wiki_vocabulary);
            match event.trigger() {
                Trigger::AtGameTime { milliseconds } if milliseconds % 1_000 == 0 => {
                    let seconds = (*milliseconds / 1_000) as i64;
                    *wiki_counts.entry(seconds).or_insert(0) += 1;
                    let numbers = wiki_numbers.entry(seconds).or_default();
                    collect_wiki_numbers(event.facts(), numbers);
                    if event
                        .facts()
                        .iter()
                        .any(|fact| matches!(fact, Fact::EventCategory { .. }))
                    {
                        wiki_categories.insert(seconds);
                    }
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

    let selected_records = records
        .iter()
        .filter(|record| record.map_name == mapping.keiframe_config)
        .collect::<Vec<_>>();
    let mut keiframe_counts = BTreeMap::new();
    let mut keiframe_time_label_mismatch_count = 0;
    let mut keiframe_text_attribute_record_count = 0;
    let mut keiframe_compound_record_count = 0;
    let mut keiframe_count_condition_record_count = 0;
    let mut keiframe_vocabulary = BTreeSet::new();
    let mut keiframe_numbers = BTreeMap::<i64, BTreeSet<u64>>::new();
    let mut keiframe_category_candidates = BTreeSet::new();
    for record in &selected_records {
        *keiframe_counts.entry(record.time_value).or_insert(0) += 1;
        if record.count_value.is_some() {
            keiframe_count_condition_record_count += 1;
        }
        if record
            .time_label
            .as_deref()
            .and_then(parse_time_label)
            .is_some_and(|seconds| seconds != record.time_value)
        {
            keiframe_time_label_mismatch_count += 1;
        }
        let event_text = nonempty(record.event_text.as_deref());
        let army_text = nonempty(record.army_text.as_deref());
        if event_text.is_some() || army_text.is_some() {
            keiframe_text_attribute_record_count += 1;
        }
        if (event_text.is_some() && army_text.is_some())
            || event_text.is_some_and(looks_compound)
            || army_text.is_some_and(looks_compound)
        {
            keiframe_compound_record_count += 1;
        }
        if event_text.is_some() {
            keiframe_category_candidates.insert(record.time_value);
        }
        for text in [event_text, army_text].into_iter().flatten() {
            collect_tokens(text, &mut keiframe_vocabulary);
            collect_numbers(text, keiframe_numbers.entry(record.time_value).or_default());
        }
    }

    let wiki_times = wiki_counts.keys().copied().collect::<BTreeSet<_>>();
    let keiframe_times = keiframe_counts.keys().copied().collect::<BTreeSet<_>>();
    let numeric_candidate_review_times = wiki_numbers
        .iter()
        .filter_map(|(time, wiki)| {
            (!wiki.is_empty()
                && keiframe_numbers
                    .get(time)
                    .is_some_and(|values| !values.is_empty()))
            .then_some(*time)
        })
        .collect::<Vec<_>>();
    let numeric_candidate_mismatch_times = numeric_candidate_review_times
        .iter()
        .copied()
        .filter(|time| wiki_numbers.get(time) != keiframe_numbers.get(time))
        .collect();
    let category_review_times = wiki_categories
        .intersection(&keiframe_category_candidates)
        .copied()
        .collect();
    let simultaneous_count_mismatches = wiki_times
        .union(&keiframe_times)
        .filter_map(|time| {
            let wiki_count = wiki_counts.get(time).copied().unwrap_or_default();
            let keiframe_count = keiframe_counts.get(time).copied().unwrap_or_default();
            (wiki_count != keiframe_count).then_some(SimultaneousCountDiff {
                time_value: *time,
                wiki_count,
                keiframe_count,
            })
        })
        .collect();
    let shared_vocabulary_token_count = wiki_vocabulary.intersection(&keiframe_vocabulary).count();
    let wiki_only_vocabulary_token_count = wiki_vocabulary.difference(&keiframe_vocabulary).count();
    let keiframe_only_vocabulary_token_count =
        keiframe_vocabulary.difference(&wiki_vocabulary).count();

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
        keiframe_time_label_mismatch_count,
        wiki_structured_fact_count,
        wiki_multi_fact_event_count,
        keiframe_text_attribute_record_count,
        keiframe_compound_record_count,
        keiframe_count_condition_record_count,
        numeric_candidate_review_times,
        numeric_candidate_mismatch_times,
        category_review_times,
        simultaneous_count_mismatches,
        shared_vocabulary_token_count,
        wiki_only_vocabulary_token_count,
        keiframe_only_vocabulary_token_count,
        unsupported_runtime_condition: mapping.map_id == "malwarfare",
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn looks_compound(value: &str) -> bool {
    ['+', '/', '&', '→', '、']
        .iter()
        .any(|separator| value.contains(*separator))
}

fn parse_time_label(value: &str) -> Option<i64> {
    let (minutes, seconds) = value.trim().split_once(':')?;
    let minutes = minutes.parse::<i64>().ok()?;
    let seconds = seconds.parse::<i64>().ok()?;
    (minutes >= 0 && (0..60).contains(&seconds)).then_some(minutes * 60 + seconds)
}

fn collect_wiki_vocabulary(facts: &[Fact], output: &mut BTreeSet<String>) {
    for fact in facts {
        match fact {
            Fact::EventCategory { value } => collect_tokens(&format!("{value:?}"), output),
            Fact::Wave { .. } | Fact::WaveExpression { .. } => collect_tokens("波次", output),
            Fact::Location { value } => match value {
                LocationSpec::Single { name } => collect_tokens(name, output),
                LocationSpec::All { names } => {
                    for name in names {
                        collect_tokens(name, output);
                    }
                }
                LocationSpec::Any { options } => {
                    for option in options {
                        collect_tokens(&option.name, output);
                    }
                }
            },
            Fact::Target { value }
            | Fact::Route { value }
            | Fact::ScaleExpression { value }
            | Fact::TechExpression { value }
            | Fact::Composition { value }
            | Fact::Probability { value } => collect_tokens(value, output),
            Fact::Health { .. } => collect_tokens("生命值", output),
            Fact::Shield { .. } => collect_tokens("护盾值", output),
            Fact::UnitCount { unit, .. } => collect_tokens(&format!("{unit:?}"), output),
            Fact::Count { subject, value } => {
                collect_tokens(subject, output);
                collect_tokens(value, output);
            }
            Fact::ScaleLevel { .. } => collect_tokens("规模", output),
            Fact::TechLevel { .. } => collect_tokens("科技", output),
            Fact::Detail { label, value } => {
                collect_tokens(label, output);
                collect_tokens(value, output);
            }
            Fact::MutatorContext {
                display_name,
                label,
                value,
                ..
            } => {
                collect_tokens(display_name, output);
                collect_tokens(label, output);
                collect_tokens(value, output);
            }
        }
    }
}

fn collect_wiki_numbers(facts: &[Fact], output: &mut BTreeSet<u64>) {
    for fact in facts {
        match fact {
            Fact::Health { value } | Fact::Shield { value } => {
                output.insert(u64::from(*value));
            }
            Fact::UnitCount { value, .. } => {
                output.insert(u64::from(*value));
            }
            Fact::ScaleLevel { value } | Fact::TechLevel { value } => {
                output.insert(u64::from(*value));
            }
            Fact::Count { value, .. }
            | Fact::ScaleExpression { value }
            | Fact::TechExpression { value }
            | Fact::Composition { value }
            | Fact::Probability { value } => collect_numbers(value, output),
            _ => {}
        }
    }
}

fn collect_tokens(value: &str, output: &mut BTreeSet<String>) {
    let mut ascii = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            ascii.push(character.to_ascii_lowercase());
        } else {
            if !ascii.is_empty() {
                output.insert(std::mem::take(&mut ascii));
            }
            if character.is_alphanumeric() {
                output.insert(character.to_string());
            }
        }
    }
    if !ascii.is_empty() {
        output.insert(ascii);
    }
}

fn collect_numbers(value: &str, output: &mut BTreeSet<u64>) {
    for token in value.split(|character: char| !character.is_ascii_digit()) {
        if let Ok(number) = token.parse() {
            output.insert(number);
        }
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
