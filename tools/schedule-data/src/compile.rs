use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

use sc2_copilot_core::{EventCategory, RuntimeSupport, ScheduleCatalog, UnsupportedReason};
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
            let source_row_count = table.rows.len().saturating_sub(1);
            let classified_rows = validate_table_coverage(
                plan.name,
                table_plan.table_index,
                source_row_count,
                &compiled,
            )?;

            classified_row_count += classified_rows;
            event_count += compiled.events.len();
            relevant_table_count += 1;
            events.extend(compiled.events);
            coverage_report.push(json!({
                "map_id": plan.id,
                "display_name": plan.name,
                "table_index": table_plan.table_index,
                "runtime_support": table_plan.runtime_support(),
                "source_row_count": source_row_count,
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
        let mut pending_events = Vec::new();
        let mut row_has_unparsed_expression = false;

        for &column_index in &time_columns {
            let value = row
                .get(column_index)
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            if value.is_empty() || value == "-" {
                continue;
            }

            let (trigger, stage_id) = match plan.policy {
                Policy::Absolute { selector, .. } => {
                    let candidate = match selector {
                        TimeSelector::EmbeddedGameTime => match value.strip_prefix("游戏时间为")
                        {
                            Some(candidate) => candidate,
                            None => continue,
                        },
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

            pending_events.push((column_index, trigger, stage_id));
        }

        if pending_events.is_empty() || row_has_unparsed_expression {
            unsupported_rows.push(unsupported_row(
                row_index,
                UnsupportedReason::SourceExpressionUnsupported,
            ));
            continue;
        }

        let trigger_columns = pending_events
            .iter()
            .map(|(column_index, _, _)| *column_index)
            .collect::<Vec<_>>();
        let facts = compile_facts(
            map,
            table.table_index,
            row_index,
            &row,
            headers,
            &trigger_columns,
            plan.category(),
        )?;
        for (column_index, trigger, stage_id) in pending_events {
            let variant_id = plan.variant_for(headers.get(column_index).map(String::as_str));
            let runtime_support = if variant_id.is_some() || stage_id.is_some() {
                RuntimeSupport::ManualContext
            } else {
                plan.runtime_support()
            };
            let event_id = format!(
                "{}-t{}-r{}-c{}",
                map.id, table.table_index, row_index, column_index
            );
            events.push(json!({
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
        handled_rows.push(row_index);
    }

    Ok(CompiledTable {
        events,
        handled_rows,
        unsupported_rows,
    })
}

fn validate_table_coverage(
    map: &str,
    table_index: usize,
    source_row_count: usize,
    compiled: &CompiledTable,
) -> Result<usize, CompileError> {
    let expected = (1..=source_row_count).collect::<BTreeSet<_>>();
    let handled = compiled
        .handled_rows
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let unsupported = compiled
        .unsupported_rows
        .iter()
        .filter_map(|row| row["row_index"].as_u64())
        .map(|row_index| row_index as usize)
        .collect::<BTreeSet<_>>();
    let overlap = handled
        .intersection(&unsupported)
        .copied()
        .collect::<Vec<_>>();
    let classified = handled
        .union(&unsupported)
        .copied()
        .collect::<BTreeSet<_>>();
    let missing = expected
        .difference(&classified)
        .copied()
        .collect::<Vec<_>>();
    let unexpected = classified
        .difference(&expected)
        .copied()
        .collect::<Vec<_>>();
    if !overlap.is_empty() || !missing.is_empty() || !unexpected.is_empty() {
        return Err(CompileError::InvalidRowCoverage {
            map: map.to_owned(),
            table_index,
            overlap,
            missing,
            unexpected,
        });
    }
    Ok(classified.len())
}

fn compile_facts(
    map: &MapPlan,
    table_index: usize,
    row_index: usize,
    row: &[String],
    headers: &[String],
    trigger_columns: &[usize],
    category: EventCategory,
) -> Result<Vec<Value>, CompileError> {
    let mut facts = vec![json!({ "kind": "event_category", "value": category })];

    for (column_index, (header, value)) in headers.iter().zip(row).enumerate() {
        let value = value.trim();
        if value.is_empty() || value == "-" || trigger_columns.contains(&column_index) {
            continue;
        }
        if is_known_expanded_duplicate_column(map, table_index, column_index) {
            continue;
        }

        if is_wave_header(header) {
            if let Some((number, branch)) = wave_number(value) {
                facts.push(json!({ "kind": "wave", "number": number, "branch": branch }));
            } else {
                facts.push(json!({ "kind": "wave_expression", "value": value }));
            }
        } else if is_location_header(header) {
            facts.push(json!({ "kind": "location", "value": location_value(value) }));
        } else if header.contains("进攻路径") || header == "前进目标" {
            facts.push(json!({ "kind": "route", "value": value }));
        } else if header.contains("目标") {
            facts.push(json!({ "kind": "target", "value": value }));
        } else if header.contains("规模") {
            if let Ok(value) = value.parse::<u8>() {
                facts.push(json!({ "kind": "scale_level", "value": value }));
            } else {
                facts.push(json!({ "kind": "scale_expression", "value": value }));
            }
        } else if header.contains("科技") {
            if let Ok(value) = value.parse::<u8>() {
                facts.push(json!({ "kind": "tech_level", "value": value }));
            } else {
                facts.push(json!({ "kind": "tech_expression", "value": value }));
            }
        } else if header.contains("生命值") {
            if let Ok(value) = value.parse::<u32>() {
                facts.push(json!({ "kind": "health", "value": value }));
            } else {
                return Err(CompileError::InvalidFactValue {
                    map: map.name.to_owned(),
                    table_index,
                    row_index,
                    column_index,
                    header: header.to_owned(),
                    value: value.to_owned(),
                });
            }
        } else if header.contains("护盾") {
            if let Ok(value) = value.parse::<u32>() {
                facts.push(json!({ "kind": "shield", "value": value }));
            } else {
                return Err(CompileError::InvalidFactValue {
                    map: map.name.to_owned(),
                    table_index,
                    row_index,
                    column_index,
                    header: header.to_owned(),
                    value: value.to_owned(),
                });
            }
        } else if header.contains("列车节数")
            && let Ok(value) = value.parse::<u16>()
        {
            facts.push(json!({
                "kind": "unit_count",
                "unit": "train_car",
                "value": value,
            }));
        } else if is_count_header(header) {
            facts.push(json!({ "kind": "count", "subject": header, "value": value }));
        } else if header.contains("混合体") || header == "补充" {
            facts.push(json!({ "kind": "composition", "value": value }));
        } else if header.contains("概率") {
            facts.push(json!({ "kind": "probability", "value": value }));
        } else if header.contains("Polarity") && header.contains("可造成伤害玩家") {
            facts.push(json!({
                "kind": "mutator_context",
                "mutator_id": "polarity",
                "display_name": "极性不定",
                "label": "可造成伤害玩家",
                "value": value,
            }));
        } else if is_detail_header(header) {
            facts.push(json!({
                "kind": "detail",
                "label": header,
                "value": value,
            }));
        } else {
            return Err(CompileError::UnknownFactColumn {
                map: map.name.to_owned(),
                table_index,
                row_index,
                column_index,
                header: header.to_owned(),
            });
        }
    }

    Ok(facts)
}

fn is_known_expanded_duplicate_column(
    map: &MapPlan,
    table_index: usize,
    column_index: usize,
) -> bool {
    map.id == "lock-and-load" && table_index == 2 && column_index == 7
}

fn is_wave_header(header: &str) -> bool {
    header.contains("波次") || header == "夜晚次数"
}

fn is_location_header(header: &str) -> bool {
    header.contains("刷新位置")
        || header.contains("刷新点")
        || header.contains("红点位置")
        || matches!(
            header,
            "位置"
                | "停泊湾"
                | "时空航道"
                | "反奖励波次位置"
                | "生成位置"
                | "出发位置"
                | "刷新区域"
                | "构造体区域"
                | "降落点"
                | "触发位置"
                | "空投区域"
        )
}

fn is_count_header(header: &str) -> bool {
    header.contains("数量")
        || header.contains("数目")
        || header.contains("个数")
        || header.contains("节数")
}

fn is_detail_header(header: &str) -> bool {
    matches!(
        header,
        "类型" | "进攻方式" | "集结时间" | "等待时间" | "出现间隔"
    ) || header.contains("速度")
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

fn unsupported_row(row_index: usize, reason: UnsupportedReason) -> Value {
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
    fn runtime_support(self) -> RuntimeSupport {
        match self.policy {
            Policy::Absolute { variant, .. } => match variant {
                VariantMode::None => RuntimeSupport::Automatic,
                VariantMode::Fixed(_) | VariantMode::SpeciesHeader => RuntimeSupport::ManualContext,
            },
            Policy::StageNight { .. }
            | Policy::StageRemaining { .. }
            | Policy::StageCradle { .. } => RuntimeSupport::ManualContext,
            Policy::Unsupported(_) => RuntimeSupport::Unsupported,
        }
    }

    fn category(self) -> EventCategory {
        match self.policy {
            Policy::Absolute { category, .. }
            | Policy::StageNight { category }
            | Policy::StageRemaining { category }
            | Policy::StageCradle { category } => category,
            Policy::Unsupported(_) => EventCategory::AttackWave,
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
        category: EventCategory,
        selector: TimeSelector,
        variant: VariantMode,
    },
    StageNight {
        category: EventCategory,
    },
    StageRemaining {
        category: EventCategory,
    },
    StageCradle {
        category: EventCategory,
    },
    Unsupported(UnsupportedReason),
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

fn absolute(table_index: usize, category: EventCategory) -> TablePlan {
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
    category: EventCategory,
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

fn unsupported(table_index: usize, reason: UnsupportedReason) -> TablePlan {
    TablePlan {
        table_index,
        policy: Policy::Unsupported(reason),
    }
}

const ATTACK_WAVE: EventCategory = EventCategory::AttackWave;
const MAIN_OBJECTIVE: EventCategory = EventCategory::MainObjective;
const BONUS_OBJECTIVE: EventCategory = EventCategory::BonusObjective;
const AMBIGUOUS_CLOCK: UnsupportedReason = UnsupportedReason::AmbiguousClock;
const CONDITION_UNAVAILABLE: UnsupportedReason = UnsupportedReason::ConditionUnavailable;
const DUPLICATE_SUMMARY: UnsupportedReason = UnsupportedReason::DuplicateSummary;
const SOURCE_EXPRESSION_UNSUPPORTED: UnsupportedReason =
    UnsupportedReason::SourceExpressionUnsupported;
const SUPPORTING_TABLE: UnsupportedReason = UnsupportedReason::SupportingTableNoIndependentTrigger;
const VISUAL_STATE_REQUIRED: UnsupportedReason = UnsupportedReason::VisualStateRequired;

fn map_plans() -> Vec<MapPlan> {
    vec![
        MapPlan {
            id: "oblivion-express",
            name: "湮灭快车",
            variants: vec![],
            tables: vec![
                absolute(0, MAIN_OBJECTIVE),
                unsupported(1, SUPPORTING_TABLE),
                unsupported(2, DUPLICATE_SUMMARY),
                absolute(3, ATTACK_WAVE),
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
                absolute(0, MAIN_OBJECTIVE),
                absolute_variant(3, ATTACK_WAVE, "layout-a"),
                absolute_variant(4, ATTACK_WAVE, "layout-b"),
            ],
        },
        MapPlan {
            id: "void-launch",
            name: "虚空降临",
            variants: vec![],
            tables: vec![
                absolute(1, MAIN_OBJECTIVE),
                absolute(2, BONUS_OBJECTIVE),
                absolute(3, ATTACK_WAVE),
            ],
        },
        MapPlan {
            id: "rifts-to-korhal",
            name: "克哈裂痕",
            variants: vec![],
            tables: vec![
                unsupported(0, CONDITION_UNAVAILABLE),
                absolute(3, ATTACK_WAVE),
                unsupported(4, CONDITION_UNAVAILABLE),
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
                absolute_variant(3, ATTACK_WAVE, "layout-a"),
                unsupported(4, SUPPORTING_TABLE),
                absolute_variant(5, ATTACK_WAVE, "layout-b"),
                unsupported(6, SUPPORTING_TABLE),
            ],
        },
        MapPlan {
            id: "lock-and-load",
            name: "天界封锁",
            variants: vec![],
            tables: vec![absolute(2, ATTACK_WAVE), unsupported(3, AMBIGUOUS_CLOCK)],
        },
        MapPlan {
            id: "chain-of-ascension",
            name: "升格之链",
            variants: vec![],
            tables: vec![
                absolute(0, MAIN_OBJECTIVE),
                unsupported(1, CONDITION_UNAVAILABLE),
                unsupported(2, SUPPORTING_TABLE),
                absolute(3, MAIN_OBJECTIVE),
                absolute(5, ATTACK_WAVE),
            ],
        },
        MapPlan {
            id: "the-vermillion-problem",
            name: "熔火危机",
            variants: vec![],
            tables: vec![
                unsupported(0, SOURCE_EXPRESSION_UNSUPPORTED),
                absolute(3, ATTACK_WAVE),
                unsupported(4, SUPPORTING_TABLE),
                unsupported(5, SUPPORTING_TABLE),
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
                absolute(2, ATTACK_WAVE),
                unsupported(3, CONDITION_UNAVAILABLE),
                unsupported(5, DUPLICATE_SUMMARY),
                unsupported(6, DUPLICATE_SUMMARY),
                unsupported(7, DUPLICATE_SUMMARY),
                unsupported(8, DUPLICATE_SUMMARY),
                unsupported(9, DUPLICATE_SUMMARY),
                absolute(10, MAIN_OBJECTIVE),
                TablePlan {
                    table_index: 11,
                    policy: Policy::Absolute {
                        category: ATTACK_WAVE,
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
                unsupported(1, CONDITION_UNAVAILABLE),
                unsupported(2, CONDITION_UNAVAILABLE),
                TablePlan {
                    table_index: 3,
                    policy: Policy::Absolute {
                        category: MAIN_OBJECTIVE,
                        selector: TimeSelector::AllAfterFirst,
                        variant: VariantMode::None,
                    },
                },
                absolute(4, ATTACK_WAVE),
                absolute(10, ATTACK_WAVE),
                unsupported(11, AMBIGUOUS_CLOCK),
                unsupported(12, AMBIGUOUS_CLOCK),
                unsupported(13, AMBIGUOUS_CLOCK),
                unsupported(14, AMBIGUOUS_CLOCK),
                unsupported(15, SUPPORTING_TABLE),
                unsupported(16, AMBIGUOUS_CLOCK),
                unsupported(17, AMBIGUOUS_CLOCK),
                unsupported(18, AMBIGUOUS_CLOCK),
                unsupported(19, AMBIGUOUS_CLOCK),
                unsupported(20, AMBIGUOUS_CLOCK),
                unsupported(21, AMBIGUOUS_CLOCK),
            ],
        },
        MapPlan {
            id: "dead-of-night",
            name: "亡者之夜",
            variants: vec![],
            tables: vec![
                unsupported(0, SOURCE_EXPRESSION_UNSUPPORTED),
                unsupported(1, SOURCE_EXPRESSION_UNSUPPORTED),
                unsupported(2, SUPPORTING_TABLE),
                TablePlan {
                    table_index: 3,
                    policy: Policy::StageNight {
                        category: ATTACK_WAVE,
                    },
                },
                TablePlan {
                    table_index: 5,
                    policy: Policy::StageNight {
                        category: ATTACK_WAVE,
                    },
                },
                TablePlan {
                    table_index: 7,
                    policy: Policy::StageNight {
                        category: ATTACK_WAVE,
                    },
                },
                TablePlan {
                    table_index: 9,
                    policy: Policy::StageNight {
                        category: ATTACK_WAVE,
                    },
                },
                TablePlan {
                    table_index: 11,
                    policy: Policy::StageNight {
                        category: ATTACK_WAVE,
                    },
                },
                TablePlan {
                    table_index: 14,
                    policy: Policy::StageNight {
                        category: ATTACK_WAVE,
                    },
                },
                TablePlan {
                    table_index: 15,
                    policy: Policy::StageRemaining {
                        category: ATTACK_WAVE,
                    },
                },
                unsupported(16, SUPPORTING_TABLE),
            ],
        },
        MapPlan {
            id: "scythe-of-amon",
            name: "黑暗杀星",
            variants: vec![],
            tables: vec![absolute(5, BONUS_OBJECTIVE), absolute(6, ATTACK_WAVE)],
        },
        MapPlan {
            id: "malwarfare",
            name: "净网行动",
            variants: vec![],
            tables: vec![
                unsupported(4, AMBIGUOUS_CLOCK),
                unsupported(5, AMBIGUOUS_CLOCK),
                TablePlan {
                    table_index: 6,
                    policy: Policy::Absolute {
                        category: ATTACK_WAVE,
                        selector: TimeSelector::EmbeddedGameTime,
                        variant: VariantMode::None,
                    },
                },
                unsupported(7, VISUAL_STATE_REQUIRED),
                unsupported(8, VISUAL_STATE_REQUIRED),
                unsupported(9, VISUAL_STATE_REQUIRED),
                unsupported(10, VISUAL_STATE_REQUIRED),
                unsupported(11, VISUAL_STATE_REQUIRED),
                unsupported(12, VISUAL_STATE_REQUIRED),
                unsupported(13, VISUAL_STATE_REQUIRED),
                unsupported(14, VISUAL_STATE_REQUIRED),
            ],
        },
        MapPlan {
            id: "part-and-parcel",
            name: "聚铁成兵",
            variants: vec![],
            tables: vec![absolute(7, ATTACK_WAVE)],
        },
        MapPlan {
            id: "cradle-of-death",
            name: "死亡摇篮",
            variants: vec![],
            tables: vec![
                unsupported(0, SUPPORTING_TABLE),
                absolute(4, ATTACK_WAVE),
                unsupported(5, SUPPORTING_TABLE),
                TablePlan {
                    table_index: 6,
                    policy: Policy::StageCradle {
                        category: ATTACK_WAVE,
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
    #[error(
        "map {map} table {table_index} row {row_index} column {column_index} has an unmodeled fact header {header:?}"
    )]
    UnknownFactColumn {
        map: String,
        table_index: usize,
        row_index: usize,
        column_index: usize,
        header: String,
    },
    #[error(
        "map {map} table {table_index} row {row_index} column {column_index} has invalid {header:?} value {value:?}"
    )]
    InvalidFactValue {
        map: String,
        table_index: usize,
        row_index: usize,
        column_index: usize,
        header: String,
        value: String,
    },
    #[error(
        "map {map} table {table_index} has invalid row coverage (overlap: {overlap:?}, missing: {missing:?}, unexpected: {unexpected:?})"
    )]
    InvalidRowCoverage {
        map: String,
        table_index: usize,
        overlap: Vec<usize>,
        missing: Vec<usize>,
        unexpected: Vec<usize>,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Catalog(#[from] sc2_copilot_core::CatalogError),
}
