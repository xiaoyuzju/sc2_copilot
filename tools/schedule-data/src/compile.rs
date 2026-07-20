use std::{collections::HashMap, path::Path};

use sc2_copilot_core::ScheduleCatalog;
use serde_json::{Value, json};
use thiserror::Error;

use crate::{Manifest, Snapshot, SnapshotError, SnapshotTable, read_json, validate_snapshot_batch};

#[derive(Debug)]
pub struct CompileOutput {
    pub catalog_json: Vec<u8>,
    pub coverage_json: Vec<u8>,
    pub report: CompileReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileReport {
    pub map_count: usize,
    pub event_count: usize,
    pub relevant_table_count: usize,
    pub classified_row_count: usize,
    pub unclassified_row_count: usize,
}

pub fn compile_snapshot_batch(batch_dir: &Path) -> Result<CompileOutput, CompileError> {
    validate_snapshot_batch(batch_dir)?;

    let batch = batch_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CompileError::InvalidBatchPath)?;
    let manifest: Manifest = read_json(&batch_dir.join("manifest.json"))?;
    let plans = map_plans();
    let mut maps = Vec::with_capacity(plans.len());
    let mut coverage_report = Vec::new();
    let mut event_count = 0;
    let mut relevant_table_count = 0;
    let mut classified_row_count = 0;

    for plan in &plans {
        let entry = manifest
            .maps
            .iter()
            .find(|entry| entry.name == plan.name)
            .ok_or_else(|| CompileError::MissingMap(plan.name.to_owned()))?;
        let snapshot: Snapshot = read_json(&batch_dir.join(&entry.file))?;
        let snapshot_path = format!("data/sources/huiji/{batch}/{}", entry.file);
        let mut events = Vec::new();
        let mut coverage = Vec::new();

        for table_plan in &plan.tables {
            let table = snapshot
                .tables
                .get(table_plan.table_index)
                .filter(|table| table.table_index == table_plan.table_index)
                .ok_or_else(|| CompileError::MissingTable {
                    map: plan.name.to_owned(),
                    table_index: table_plan.table_index,
                })?;
            let compiled =
                compile_table(plan, table_plan, table, &snapshot, batch, &snapshot_path)?;

            classified_row_count += table.rows.len().saturating_sub(1);
            event_count += compiled.events.len();
            relevant_table_count += 1;
            events.extend(compiled.events);
            coverage_report.push(json!({
                "map_id": plan.id,
                "display_name": plan.name,
                "table_index": table_plan.table_index,
                "runtime_support": table_plan.runtime_support(),
                "handled_rows": compiled.handled_rows,
                "unsupported_rows": compiled.unsupported_rows,
            }));
            coverage.push(json!({
                "source_url": snapshot.source_url,
                "snapshot_batch": batch,
                "snapshot_path": snapshot_path,
                "table_index": table_plan.table_index,
                "runtime_support": table_plan.runtime_support(),
                "unsupported_rows": compiled.unsupported_rows,
            }));
        }

        maps.push(json!({
            "map_id": plan.id,
            "display_name": plan.name,
            "variants": plan.variants.iter().map(|variant| json!({
                "variant_id": variant.id,
                "display_name": variant.display_name,
            })).collect::<Vec<_>>(),
            "events": events,
            "coverage": coverage,
        }));
    }

    let catalog = json!({
        "schema_version": 1,
        "snapshot_batch": batch,
        "maps": maps,
    });
    let catalog_json = serde_json::to_vec_pretty(&catalog)?;
    ScheduleCatalog::from_json(&catalog_json)?;

    let coverage_json = serde_json::to_vec_pretty(&json!({
        "schema_version": 1,
        "snapshot_batch": batch,
        "map_count": plans.len(),
        "relevant_table_count": relevant_table_count,
        "classified_row_count": classified_row_count,
        "unclassified_row_count": 0,
        "tables": coverage_report,
    }))?;

    Ok(CompileOutput {
        catalog_json,
        coverage_json,
        report: CompileReport {
            map_count: plans.len(),
            event_count,
            relevant_table_count,
            classified_row_count,
            unclassified_row_count: 0,
        },
    })
}

fn compile_table(
    map: &MapPlan,
    plan: &TablePlan,
    table: &SnapshotTable,
    snapshot: &Snapshot,
    batch: &str,
    snapshot_path: &str,
) -> Result<CompiledTable, CompileError> {
    if let Policy::Unsupported(reason) = plan.policy {
        let unsupported_rows = (1..table.rows.len())
            .map(|row_index| unsupported_row(row_index, reason))
            .collect::<Vec<_>>();
        return Ok(CompiledTable {
            events: Vec::new(),
            handled_rows: Vec::new(),
            unsupported_rows,
        });
    }

    let grid = expand_table(table);
    let headers = grid.first().ok_or_else(|| CompileError::EmptyTable {
        map: map.name.to_owned(),
        table_index: table.table_index,
    })?;
    let time_columns = match plan.policy {
        Policy::Absolute { selector, .. } => select_time_columns(headers, selector),
        Policy::StageNight { .. } | Policy::StageRemaining { .. } => headers
            .iter()
            .enumerate()
            .filter_map(|(index, header)| header.contains("触发时间").then_some(index))
            .collect(),
        Policy::StageCradle { .. } => vec![1],
        Policy::Unsupported(_) => unreachable!(),
    };
    if time_columns.is_empty() {
        return Err(CompileError::MissingTimeColumn {
            map: map.name.to_owned(),
            table_index: table.table_index,
        });
    }

    let mut events = Vec::new();
    let mut handled_rows = Vec::new();
    let mut unsupported_rows = Vec::new();

    for row_index in 1..table.rows.len() {
        let row = grid.get(row_index).cloned().unwrap_or_default();
        let mut row_events = Vec::new();
        let mut row_has_unparsed_expression = false;

        for &column_index in &time_columns {
            let value = row
                .get(column_index)
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            if value.is_empty() {
                continue;
            }

            let (trigger, stage_id) = match plan.policy {
                Policy::Absolute { selector, .. } => {
                    let candidate = match selector {
                        TimeSelector::EmbeddedGameTime => {
                            value.strip_prefix("游戏时间为").unwrap_or(value)
                        }
                        _ => value,
                    };
                    match parse_time_expression(candidate) {
                        Some(time) => (time.as_game_trigger(), None),
                        None => {
                            row_has_unparsed_expression = true;
                            continue;
                        }
                    }
                }
                Policy::StageNight { .. } => {
                    let Some(night) = row
                        .first()
                        .and_then(|value| value.trim().parse::<u16>().ok())
                    else {
                        row_has_unparsed_expression = true;
                        continue;
                    };
                    let Some(time) = parse_time_expression(value) else {
                        row_has_unparsed_expression = true;
                        continue;
                    };
                    let Some(milliseconds) = time.exact_milliseconds() else {
                        row_has_unparsed_expression = true;
                        continue;
                    };
                    let stage_id = format!("night-{night}");
                    (
                        json!({
                            "kind": "at_stage_elapsed",
                            "stage_id": stage_id,
                            "milliseconds": milliseconds,
                        }),
                        Some(stage_id),
                    )
                }
                Policy::StageRemaining { .. } => {
                    row_has_unparsed_expression = true;
                    continue;
                }
                Policy::StageCradle { .. } => {
                    let Some((stage_id, milliseconds)) = parse_cradle_stage_time(value) else {
                        row_has_unparsed_expression = true;
                        continue;
                    };
                    (
                        json!({
                            "kind": "at_stage_elapsed",
                            "stage_id": stage_id,
                            "milliseconds": milliseconds,
                        }),
                        Some(stage_id),
                    )
                }
                Policy::Unsupported(_) => unreachable!(),
            };

            let variant_id = plan.variant_for(headers.get(column_index).map(String::as_str));
            let runtime_support = if variant_id.is_some() || stage_id.is_some() {
                "manual_context"
            } else {
                plan.runtime_support()
            };
            let event_id = format!(
                "{}-t{}-r{}-c{}",
                map.id, table.table_index, row_index, column_index
            );
            let facts = compile_facts(&row, headers, plan.category());
            row_events.push(json!({
                "map_id": map.id,
                "variant_id": variant_id,
                "event_id": event_id,
                "trigger": trigger,
                "facts": facts,
                "source_refs": [{
                    "source_url": snapshot.source_url,
                    "snapshot_batch": batch,
                    "snapshot_path": snapshot_path,
                    "table_index": table.table_index,
                    "row_index": row_index,
                }],
                "runtime_support": runtime_support,
            }));
        }

        if row_events.is_empty() || row_has_unparsed_expression {
            unsupported_rows.push(unsupported_row(row_index, "source_expression_unsupported"));
        }
        if !row_events.is_empty() {
            handled_rows.push(row_index);
            events.extend(row_events);
        }
    }

    Ok(CompiledTable {
        events,
        handled_rows,
        unsupported_rows,
    })
}

fn compile_facts(row: &[String], headers: &[String], category: &str) -> Vec<Value> {
    let mut facts = vec![json!({ "kind": "event_category", "value": category })];

    if let Some((number, branch)) = row.first().and_then(|value| wave_number(value)) {
        facts.push(json!({ "kind": "wave", "number": number, "branch": branch }));
    }

    for (header, value) in headers.iter().zip(row) {
        let value = value.trim();
        if value.is_empty() || value == "-" {
            continue;
        }
        if (header.contains("刷新位置")
            || header.contains("刷新点")
            || header.contains("红点位置")
            || header == "位置")
            && !facts.iter().any(|fact| fact["kind"] == "location")
        {
            facts.push(json!({ "kind": "location", "value": location_value(value) }));
        } else if header.contains("目标") && !facts.iter().any(|fact| fact["kind"] == "target") {
            facts.push(json!({ "kind": "target", "value": value }));
        } else if header.contains("规模") {
            if let Ok(value) = value.parse::<u8>() {
                facts.push(json!({ "kind": "scale_level", "value": value }));
            }
        } else if header.contains("科技") {
            if let Ok(value) = value.parse::<u8>() {
                facts.push(json!({ "kind": "tech_level", "value": value }));
            }
        } else if header.contains("生命值") {
            if let Ok(value) = value.parse::<u32>() {
                facts.push(json!({ "kind": "health", "value": value }));
            }
        } else if header.contains("列车节数")
            && let Ok(value) = value.parse::<u16>()
        {
            facts.push(json!({
                "kind": "unit_count",
                "unit": "train_car",
                "value": value,
            }));
        }
    }

    facts
}

fn location_value(value: &str) -> Value {
    let normalized = value.replace('＆', "&");
    if normalized.contains('&') {
        return json!({
            "mode": "all",
            "names": normalized.split('&').map(str::trim).filter(|part| !part.is_empty()).collect::<Vec<_>>(),
        });
    }
    if normalized.contains('/') {
        let options = normalized
            .split('/')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| {
                let (name, weight_percent) = parse_weighted_name(part);
                json!({ "name": name, "weight_percent": weight_percent })
            })
            .collect::<Vec<_>>();
        return json!({ "mode": "any", "options": options });
    }
    json!({ "mode": "single", "name": normalized })
}

fn parse_weighted_name(value: &str) -> (&str, Option<u8>) {
    let Some(open) = value.rfind('(') else {
        return (value, None);
    };
    let Some(percent) = value[open + 1..].strip_suffix("%)") else {
        return (value, None);
    };
    match percent.parse::<u8>() {
        Ok(weight) => (value[..open].trim(), Some(weight)),
        Err(_) => (value, None),
    }
}

fn wave_number(value: &str) -> Option<(u16, Option<u16>)> {
    let value = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .collect::<String>();
    let digits = value
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    let number = digits.parse().ok()?;
    let branch = value
        .strip_prefix(&digits)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        });
    Some((number, branch))
}

fn select_time_columns(headers: &[String], selector: TimeSelector) -> Vec<usize> {
    match selector {
        TimeSelector::HeaderTime => headers
            .iter()
            .enumerate()
            .filter_map(|(index, header)| header.contains("时间").then_some(index))
            .collect(),
        TimeSelector::AllAfterFirst => (1..headers.len()).collect(),
        TimeSelector::EmbeddedGameTime => (0..headers.len()).collect(),
    }
}

fn expand_table(table: &SnapshotTable) -> Vec<Vec<String>> {
    let mut cells = HashMap::new();
    let mut width = 0;

    for (row_index, row) in table.rows.iter().enumerate() {
        let mut column_index = 0;
        for cell in row {
            while cells.contains_key(&(row_index, column_index)) {
                column_index += 1;
            }
            for row_offset in 0..cell.row_span {
                for column_offset in 0..cell.column_span {
                    cells.insert(
                        (row_index + row_offset, column_index + column_offset),
                        cell.text.clone(),
                    );
                }
            }
            column_index += cell.column_span;
            width = width.max(column_index);
        }
    }

    (0..table.rows.len())
        .map(|row_index| {
            (0..width)
                .map(|column_index| {
                    cells
                        .get(&(row_index, column_index))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect()
}

fn parse_time_expression(value: &str) -> Option<ParsedTime> {
    let value = value.trim().trim_matches(['(', ')']);
    if let Some((base, uncertainty)) = value.split_once('±') {
        let center = parse_clock(base.trim())?;
        let uncertainty = uncertainty
            .trim()
            .trim_end_matches('秒')
            .parse::<f64>()
            .ok()?;
        let uncertainty = (uncertainty * 1_000.0).round() as u64;
        return Some(ParsedTime::Window {
            earliest: center.saturating_sub(uncertainty),
            latest: center + uncertainty,
        });
    }
    if let Some(seconds) = value.strip_suffix('秒') {
        let milliseconds = (seconds.trim().parse::<f64>().ok()? * 1_000.0).round() as u64;
        return Some(ParsedTime::Exact(milliseconds));
    }
    parse_clock(value).map(ParsedTime::Exact)
}

fn parse_clock(value: &str) -> Option<u64> {
    let (minutes, seconds) = value.split_once(':')?;
    let minutes = minutes.trim().parse::<u64>().ok()?;
    let seconds = seconds.trim().parse::<f64>().ok()?;
    (seconds < 60.0).then(|| minutes * 60_000 + (seconds * 1_000.0).round() as u64)
}

fn parse_cradle_stage_time(value: &str) -> Option<(String, u64)> {
    let value = value.strip_prefix("主目标波次")?;
    let (wave, elapsed) = value.split_once("启动")?;
    let wave = wave.parse::<u16>().ok()?;
    let elapsed = elapsed.strip_suffix('时')?;
    let mut milliseconds = 0;
    let remaining = if let Some((minutes, remaining)) = elapsed.split_once('分') {
        milliseconds += minutes.parse::<u64>().ok()? * 60_000;
        remaining
    } else {
        elapsed
    };
    if let Some(seconds) = remaining.strip_suffix('秒') {
        milliseconds += seconds.parse::<u64>().ok()? * 1_000;
    } else if !remaining.is_empty() {
        return None;
    }
    Some((format!("main-objective-{wave}"), milliseconds))
}

fn unsupported_row(row_index: usize, reason: &str) -> Value {
    json!({ "row_index": row_index, "reason": reason })
}

#[derive(Debug)]
struct CompiledTable {
    events: Vec<Value>,
    handled_rows: Vec<usize>,
    unsupported_rows: Vec<Value>,
}

#[derive(Debug, Clone, Copy)]
enum ParsedTime {
    Exact(u64),
    Window { earliest: u64, latest: u64 },
}

impl ParsedTime {
    fn as_game_trigger(self) -> Value {
        match self {
            Self::Exact(milliseconds) => {
                json!({ "kind": "at_game_time", "milliseconds": milliseconds })
            }
            Self::Window { earliest, latest } => json!({
                "kind": "at_game_time_window",
                "earliest_milliseconds": earliest,
                "latest_milliseconds": latest,
            }),
        }
    }

    fn exact_milliseconds(self) -> Option<u64> {
        match self {
            Self::Exact(milliseconds) => Some(milliseconds),
            Self::Window { .. } => None,
        }
    }
}

#[derive(Debug)]
struct MapPlan {
    id: &'static str,
    name: &'static str,
    variants: Vec<VariantPlan>,
    tables: Vec<TablePlan>,
}

#[derive(Debug, Clone, Copy)]
struct VariantPlan {
    id: &'static str,
    display_name: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct TablePlan {
    table_index: usize,
    policy: Policy,
}

impl TablePlan {
    fn runtime_support(self) -> &'static str {
        match self.policy {
            Policy::Absolute { variant, .. } => match variant {
                VariantMode::None => "automatic",
                VariantMode::Fixed(_) | VariantMode::SpeciesHeader => "manual_context",
            },
            Policy::StageNight { .. }
            | Policy::StageRemaining { .. }
            | Policy::StageCradle { .. } => "manual_context",
            Policy::Unsupported(_) => "unsupported",
        }
    }

    fn category(self) -> &'static str {
        match self.policy {
            Policy::Absolute { category, .. }
            | Policy::StageNight { category }
            | Policy::StageRemaining { category }
            | Policy::StageCradle { category } => category,
            Policy::Unsupported(_) => "attack_wave",
        }
    }

    fn variant_for(self, header: Option<&str>) -> Option<&'static str> {
        match self.policy {
            Policy::Absolute {
                variant: VariantMode::Fixed(variant),
                ..
            } => Some(variant),
            Policy::Absolute {
                variant: VariantMode::SpeciesHeader,
                ..
            } => header.and_then(|header| {
                if header.contains("星灵") {
                    Some("protoss")
                } else if header.contains("人虫") {
                    Some("terran-zerg")
                } else {
                    None
                }
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Policy {
    Absolute {
        category: &'static str,
        selector: TimeSelector,
        variant: VariantMode,
    },
    StageNight {
        category: &'static str,
    },
    StageRemaining {
        category: &'static str,
    },
    StageCradle {
        category: &'static str,
    },
    Unsupported(&'static str),
}

#[derive(Debug, Clone, Copy)]
enum TimeSelector {
    HeaderTime,
    AllAfterFirst,
    EmbeddedGameTime,
}

#[derive(Debug, Clone, Copy)]
enum VariantMode {
    None,
    Fixed(&'static str),
    SpeciesHeader,
}

fn absolute(table_index: usize, category: &'static str) -> TablePlan {
    TablePlan {
        table_index,
        policy: Policy::Absolute {
            category,
            selector: TimeSelector::HeaderTime,
            variant: VariantMode::None,
        },
    }
}

fn absolute_variant(
    table_index: usize,
    category: &'static str,
    variant: &'static str,
) -> TablePlan {
    TablePlan {
        table_index,
        policy: Policy::Absolute {
            category,
            selector: TimeSelector::HeaderTime,
            variant: VariantMode::Fixed(variant),
        },
    }
}

fn unsupported(table_index: usize, reason: &'static str) -> TablePlan {
    TablePlan {
        table_index,
        policy: Policy::Unsupported(reason),
    }
}

fn map_plans() -> Vec<MapPlan> {
    vec![
        MapPlan {
            id: "oblivion-express",
            name: "湮灭快车",
            variants: vec![],
            tables: vec![
                absolute(0, "main_objective"),
                unsupported(1, "supporting_table_no_independent_trigger"),
                unsupported(2, "duplicate_summary"),
                absolute(3, "attack_wave"),
            ],
        },
        MapPlan {
            id: "void-rifts",
            name: "虚空撕裂",
            variants: vec![
                VariantPlan {
                    id: "layout-a",
                    display_name: "时间表 A",
                },
                VariantPlan {
                    id: "layout-b",
                    display_name: "时间表 B",
                },
            ],
            tables: vec![
                absolute(0, "main_objective"),
                absolute_variant(3, "attack_wave", "layout-a"),
                absolute_variant(4, "attack_wave", "layout-b"),
            ],
        },
        MapPlan {
            id: "void-launch",
            name: "虚空降临",
            variants: vec![],
            tables: vec![
                absolute(1, "main_objective"),
                absolute(2, "bonus_objective"),
                absolute(3, "attack_wave"),
            ],
        },
        MapPlan {
            id: "rifts-to-korhal",
            name: "克哈裂痕",
            variants: vec![],
            tables: vec![
                unsupported(0, "condition_unavailable"),
                absolute(3, "attack_wave"),
                unsupported(4, "condition_unavailable"),
            ],
        },
        MapPlan {
            id: "temple-of-the-past",
            name: "往日神庙",
            variants: vec![
                VariantPlan {
                    id: "layout-a",
                    display_name: "时间表 A",
                },
                VariantPlan {
                    id: "layout-b",
                    display_name: "时间表 B",
                },
            ],
            tables: vec![
                absolute_variant(3, "attack_wave", "layout-a"),
                unsupported(4, "supporting_table_no_independent_trigger"),
                absolute_variant(5, "attack_wave", "layout-b"),
                unsupported(6, "supporting_table_no_independent_trigger"),
            ],
        },
        MapPlan {
            id: "lock-and-load",
            name: "天界封锁",
            variants: vec![],
            tables: vec![
                absolute(2, "attack_wave"),
                unsupported(3, "ambiguous_clock"),
            ],
        },
        MapPlan {
            id: "chain-of-ascension",
            name: "升格之链",
            variants: vec![],
            tables: vec![
                absolute(0, "main_objective"),
                unsupported(1, "condition_unavailable"),
                unsupported(2, "supporting_table_no_independent_trigger"),
                absolute(3, "main_objective"),
                absolute(5, "attack_wave"),
            ],
        },
        MapPlan {
            id: "the-vermillion-problem",
            name: "熔火危机",
            variants: vec![],
            tables: vec![
                unsupported(0, "source_expression_unsupported"),
                absolute(3, "attack_wave"),
                unsupported(4, "supporting_table_no_independent_trigger"),
                unsupported(5, "supporting_table_no_independent_trigger"),
            ],
        },
        MapPlan {
            id: "mist-opportunities",
            name: "机会渺茫",
            variants: vec![
                VariantPlan {
                    id: "protoss",
                    display_name: "星灵敌军",
                },
                VariantPlan {
                    id: "terran-zerg",
                    display_name: "人类或异虫敌军",
                },
            ],
            tables: vec![
                absolute(2, "attack_wave"),
                unsupported(3, "condition_unavailable"),
                unsupported(5, "duplicate_summary"),
                unsupported(6, "duplicate_summary"),
                unsupported(7, "duplicate_summary"),
                unsupported(8, "duplicate_summary"),
                unsupported(9, "duplicate_summary"),
                absolute(10, "main_objective"),
                TablePlan {
                    table_index: 11,
                    policy: Policy::Absolute {
                        category: "attack_wave",
                        selector: TimeSelector::HeaderTime,
                        variant: VariantMode::SpeciesHeader,
                    },
                },
            ],
        },
        MapPlan {
            id: "miner-evacuation",
            name: "营救矿工",
            variants: vec![],
            tables: vec![
                unsupported(1, "condition_unavailable"),
                unsupported(2, "condition_unavailable"),
                TablePlan {
                    table_index: 3,
                    policy: Policy::Absolute {
                        category: "main_objective",
                        selector: TimeSelector::AllAfterFirst,
                        variant: VariantMode::None,
                    },
                },
                absolute(4, "attack_wave"),
                absolute(10, "attack_wave"),
                unsupported(11, "ambiguous_clock"),
                unsupported(12, "ambiguous_clock"),
                unsupported(13, "ambiguous_clock"),
                unsupported(14, "ambiguous_clock"),
                unsupported(15, "supporting_table_no_independent_trigger"),
                unsupported(16, "ambiguous_clock"),
                unsupported(17, "ambiguous_clock"),
                unsupported(18, "ambiguous_clock"),
                unsupported(19, "ambiguous_clock"),
                unsupported(20, "ambiguous_clock"),
                unsupported(21, "ambiguous_clock"),
            ],
        },
        MapPlan {
            id: "dead-of-night",
            name: "亡者之夜",
            variants: vec![],
            tables: vec![
                unsupported(0, "source_expression_unsupported"),
                unsupported(1, "source_expression_unsupported"),
                unsupported(2, "supporting_table_no_independent_trigger"),
                TablePlan {
                    table_index: 3,
                    policy: Policy::StageNight {
                        category: "attack_wave",
                    },
                },
                TablePlan {
                    table_index: 5,
                    policy: Policy::StageNight {
                        category: "attack_wave",
                    },
                },
                TablePlan {
                    table_index: 7,
                    policy: Policy::StageNight {
                        category: "attack_wave",
                    },
                },
                TablePlan {
                    table_index: 9,
                    policy: Policy::StageNight {
                        category: "attack_wave",
                    },
                },
                TablePlan {
                    table_index: 11,
                    policy: Policy::StageNight {
                        category: "attack_wave",
                    },
                },
                TablePlan {
                    table_index: 14,
                    policy: Policy::StageNight {
                        category: "attack_wave",
                    },
                },
                TablePlan {
                    table_index: 15,
                    policy: Policy::StageRemaining {
                        category: "attack_wave",
                    },
                },
                unsupported(16, "supporting_table_no_independent_trigger"),
            ],
        },
        MapPlan {
            id: "scythe-of-amon",
            name: "黑暗杀星",
            variants: vec![],
            tables: vec![absolute(5, "bonus_objective"), absolute(6, "attack_wave")],
        },
        MapPlan {
            id: "malwarfare",
            name: "净网行动",
            variants: vec![],
            tables: vec![
                unsupported(4, "ambiguous_clock"),
                unsupported(5, "ambiguous_clock"),
                TablePlan {
                    table_index: 6,
                    policy: Policy::Absolute {
                        category: "attack_wave",
                        selector: TimeSelector::EmbeddedGameTime,
                        variant: VariantMode::None,
                    },
                },
                unsupported(7, "visual_state_required"),
                unsupported(8, "visual_state_required"),
                unsupported(9, "visual_state_required"),
                unsupported(10, "visual_state_required"),
                unsupported(11, "visual_state_required"),
                unsupported(12, "visual_state_required"),
                unsupported(13, "visual_state_required"),
                unsupported(14, "visual_state_required"),
            ],
        },
        MapPlan {
            id: "part-and-parcel",
            name: "聚铁成兵",
            variants: vec![],
            tables: vec![absolute(7, "attack_wave")],
        },
        MapPlan {
            id: "cradle-of-death",
            name: "死亡摇篮",
            variants: vec![],
            tables: vec![
                unsupported(0, "supporting_table_no_independent_trigger"),
                absolute(4, "attack_wave"),
                unsupported(5, "supporting_table_no_independent_trigger"),
                TablePlan {
                    table_index: 6,
                    policy: Policy::StageCradle {
                        category: "attack_wave",
                    },
                },
            ],
        },
    ]
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error("snapshot batch path has no directory name")]
    InvalidBatchPath,
    #[error("snapshot batch is missing map {0}")]
    MissingMap(String),
    #[error("map {map} is missing table {table_index}")]
    MissingTable { map: String, table_index: usize },
    #[error("map {map} table {table_index} is empty")]
    EmptyTable { map: String, table_index: usize },
    #[error("map {map} table {table_index} has no configured time column")]
    MissingTimeColumn { map: String, table_index: usize },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Catalog(#[from] sc2_copilot_core::CatalogError),
}
