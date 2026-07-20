use std::{collections::HashMap, env, fs, path::Path};

use sc2_copilot_core::ScheduleCatalog;
use schedule_data::{compile_snapshot_batch, diff_keiframe, validate_snapshot_batch};
use serde::Deserialize;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let command = args.next().ok_or(
        "usage: schedule-data <validate|validate-snapshots|compile|diff-keiframe> <arguments...>",
    )?;

    if command == "validate-snapshots" {
        let batch_dir = args
            .next()
            .ok_or("usage: schedule-data validate-snapshots <batch-dir>")?;
        let stats = validate_snapshot_batch(Path::new(&batch_dir))?;
        println!(
            "validated {} maps, {} tables, {} rows and {} merged cells",
            stats.map_count, stats.table_count, stats.row_count, stats.merged_cell_count
        );
        return Ok(());
    }

    if command == "compile" {
        let batch_dir = args
            .next()
            .ok_or("usage: schedule-data compile <batch-dir> <catalog.json> <coverage.json>")?;
        let catalog_path = args
            .next()
            .ok_or("usage: schedule-data compile <batch-dir> <catalog.json> <coverage.json>")?;
        let coverage_path = args
            .next()
            .ok_or("usage: schedule-data compile <batch-dir> <catalog.json> <coverage.json>")?;
        let output = compile_snapshot_batch(Path::new(&batch_dir))?;
        fs::write(&catalog_path, output.catalog_json)?;
        fs::write(&coverage_path, output.coverage_json)?;
        println!(
            "compiled {} events from {} related tables across {} maps",
            output.report.event_count, output.report.relevant_table_count, output.report.map_count
        );
        return Ok(());
    }

    if command == "diff-keiframe" {
        let catalog_path = args.next().ok_or(
            "usage: schedule-data diff-keiframe <catalog.json> <keiframe-repo> <report.json>",
        )?;
        let keiframe_repo = args.next().ok_or(
            "usage: schedule-data diff-keiframe <catalog.json> <keiframe-repo> <report.json>",
        )?;
        let report_path = args.next().ok_or(
            "usage: schedule-data diff-keiframe <catalog.json> <keiframe-repo> <report.json>",
        )?;
        let bytes = fs::read(&catalog_path)?;
        let catalog = ScheduleCatalog::from_json(&bytes)?;
        let report = diff_keiframe(&catalog, Path::new(&keiframe_repo))?;
        if let Some(parent) = Path::new(&report_path).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
        println!(
            "compared {} Keiframe configurations at {}",
            report.configs.len(),
            report.reference_commit
        );
        return Ok(());
    }

    let path = if command == "validate" {
        args.next()
            .ok_or("usage: schedule-data validate <catalog.json>")?
    } else {
        command
    };
    let bytes = fs::read(&path)?;
    let catalog = ScheduleCatalog::from_json(&bytes)?;
    let catalog_path = fs::canonicalize(&path)?;
    let workspace_root = catalog_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or("catalog path must be under <workspace>/data/maps")?;
    validate_source_refs(&catalog, workspace_root).map_err(std::io::Error::other)?;

    println!(
        "validated {} map schedule(s) and their source references",
        catalog.map_count()
    );
    Ok(())
}

fn validate_source_refs(catalog: &ScheduleCatalog, workspace_root: &Path) -> Result<(), String> {
    let mut snapshots = HashMap::new();

    for schedule in catalog.schedules() {
        for event in schedule.events() {
            for source_ref in event.source_refs() {
                let snapshot_path = workspace_root.join(&source_ref.snapshot_path);
                let snapshot = if let Some(snapshot) = snapshots.get(&snapshot_path) {
                    snapshot
                } else {
                    let bytes = fs::read(&snapshot_path).map_err(|error| {
                        format!("cannot read {}: {error}", snapshot_path.display())
                    })?;
                    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|error| {
                        format!("invalid snapshot {}: {error}", snapshot_path.display())
                    })?;
                    snapshots.insert(snapshot_path.clone(), snapshot);
                    snapshots
                        .get(&snapshot_path)
                        .expect("snapshot was just inserted")
                };

                let source_matches = source_ref.source_url == snapshot.source_url
                    || source_ref.source_url.ends_with(&snapshot.index_name);
                if !source_matches {
                    return Err(format!(
                        "source URL for {}/{} does not match {}",
                        schedule.id(),
                        event.id(),
                        snapshot_path.display()
                    ));
                }
                if !Path::new(&source_ref.snapshot_path)
                    .components()
                    .any(|component| component.as_os_str() == source_ref.snapshot_batch.as_str())
                {
                    return Err(format!(
                        "snapshot batch for {}/{} is not present in {}",
                        schedule.id(),
                        event.id(),
                        source_ref.snapshot_path
                    ));
                }

                let table = snapshot
                    .tables
                    .iter()
                    .find(|table| table.table_index == source_ref.table_index)
                    .ok_or_else(|| {
                        format!(
                            "table {} for {}/{} is missing from {}",
                            source_ref.table_index,
                            schedule.id(),
                            event.id(),
                            snapshot_path.display()
                        )
                    })?;
                if source_ref.row_index == 0 || source_ref.row_index >= table.rows.len() {
                    return Err(format!(
                        "row {} in table {} for {}/{} is not a data row",
                        source_ref.row_index,
                        source_ref.table_index,
                        schedule.id(),
                        event.id()
                    ));
                }
            }
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct Snapshot {
    source_url: String,
    index_name: String,
    tables: Vec<SnapshotTable>,
}

#[derive(Deserialize)]
struct SnapshotTable {
    table_index: usize,
    rows: Vec<Vec<serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use sc2_copilot_core::ScheduleCatalog;

    #[test]
    fn oblivion_express_source_references_resolve_to_snapshot_rows() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog_bytes = fs::read(workspace_root.join("data/maps/catalog.json"))
            .expect("catalog fixture should be readable");
        let catalog =
            ScheduleCatalog::from_json(&catalog_bytes).expect("catalog fixture should be valid");

        super::validate_source_refs(&catalog, &workspace_root)
            .expect("every source reference should resolve");
    }

    #[test]
    fn missing_snapshot_table_is_rejected() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = ScheduleCatalog::from_json(
            r#"
            {
              "schema_version": 1,
              "maps": [{
                "map_id": "oblivion-express",
                "display_name": "湮灭快车",
                "events": [{
                  "map_id": "oblivion-express",
                  "event_id": "bad-reference",
                  "trigger": { "kind": "at_game_time", "milliseconds": 1000 },
                  "facts": [],
                  "source_refs": [{
                    "source_url": "https://starcraft.huijiwiki.com/wiki/合作任务/湮灭快车",
                    "snapshot_batch": "2026-07-21",
                    "snapshot_path": "data/sources/huiji/2026-07-21/湮灭快车.json",
                    "table_index": 99,
                    "row_index": 1
                  }],
                  "runtime_support": "automatic"
                }]
              }]
            }
            "#
            .as_bytes(),
        )
        .expect("catalog shape should be valid");

        let error = super::validate_source_refs(&catalog, &workspace_root)
            .expect_err("unknown source table must be rejected");

        assert!(error.contains("table 99"));
    }
}
