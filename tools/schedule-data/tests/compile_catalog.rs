use std::path::Path;

use sc2_copilot_core::{Fact, RuntimeSupport, ScheduleCatalog, Trigger, UnsupportedReason};
use schedule_data::compile_snapshot_batch;

#[test]
fn compiles_all_maps_and_classifies_every_related_table_row() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = compile_snapshot_batch(&workspace_root.join("data/sources/huiji/2026-07-21"))
        .expect("checked-in snapshots should compile");

    assert_eq!(output.report.map_count, 15);
    assert_eq!(output.report.relevant_table_count, 82);
    assert_eq!(output.report.unclassified_row_count, 0);
    assert!(output.report.event_count > 100);

    let coverage: serde_json::Value = serde_json::from_slice(&output.coverage_json)
        .expect("coverage report should be valid JSON");
    for table in coverage["tables"]
        .as_array()
        .expect("coverage tables should be an array")
    {
        let source_row_count = table["source_row_count"]
            .as_u64()
            .expect("coverage should expose the source row count")
            as usize;
        let handled = table["handled_rows"]
            .as_array()
            .expect("handled rows should be an array")
            .iter()
            .map(|row| row.as_u64().expect("handled row should be an index"))
            .collect::<std::collections::BTreeSet<_>>();
        let unsupported = table["unsupported_rows"]
            .as_array()
            .expect("unsupported rows should be an array")
            .iter()
            .map(|row| {
                row["row_index"]
                    .as_u64()
                    .expect("unsupported row should have an index")
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert!(
            handled.is_disjoint(&unsupported),
            "a source row cannot be both handled and unsupported: {table}"
        );
        assert_eq!(
            handled.len() + unsupported.len(),
            source_row_count,
            "every source row must have exactly one coverage classification: {table}"
        );
    }

    let catalog = ScheduleCatalog::from_json(&output.catalog_json)
        .expect("compiler output should satisfy the runtime schema");
    assert_eq!(catalog.map_count(), 15);
    assert_eq!(catalog.snapshot_batch(), "2026-07-21");

    let rifts = catalog
        .schedule_for("void-rifts", Some("layout-a"))
        .expect("manual rift layout should exist");
    assert_eq!(rifts.variants().len(), 2);
    assert!(rifts.events().iter().any(|event| {
        event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::Route { .. }))
    }));

    let express = catalog
        .schedule_for("oblivion-express", None)
        .expect("Oblivion Express should exist");
    assert!(express.events().iter().any(|event| {
        event.facts().iter().any(|fact| {
            matches!(
                fact,
                Fact::MutatorContext { mutator_id, .. } if mutator_id == "polarity"
            )
        })
    }));

    let lock_and_load = catalog
        .schedule_for("lock-and-load", None)
        .expect("Lock and Load should exist");
    assert!(lock_and_load.events().iter().all(|event| {
        event
            .facts()
            .iter()
            .all(|fact| !matches!(fact, Fact::Detail { label, .. } if label == "未命名字段"))
    }));

    let night = catalog
        .schedule_for("dead-of-night", None)
        .expect("Dead of Night should exist");
    assert!(night.events().iter().any(|event| {
        event.runtime_support() == RuntimeSupport::ManualContext
            && matches!(event.trigger(), Trigger::AtStageElapsed { .. })
    }));

    let malwarfare = catalog
        .schedule_for("malwarfare", None)
        .expect("Malwarfare should exist");
    assert!(malwarfare.coverage().iter().any(|coverage| {
        coverage.runtime_support() == RuntimeSupport::Unsupported
            && coverage
                .unsupported_rows()
                .iter()
                .any(|row| row.reason() == UnsupportedReason::VisualStateRequired)
    }));
    let game_time_event = malwarfare
        .events()
        .iter()
        .find(|event| event.id() == "malwarfare-t6-r1-c1")
        .expect("explicit Malwarfare game-time row should compile");
    assert!(
        game_time_event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::Wave { number: 1, .. }))
    );
    assert!(
        game_time_event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::Location { .. }))
    );
    assert!(
        game_time_event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::ScaleLevel { value: 1 }))
    );
    assert!(
        game_time_event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::TechLevel { value: 1 }))
    );
    assert!(
        game_time_event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::Target { .. }))
    );

    let unsupported_source_rows = coverage["tables"]
        .as_array()
        .expect("coverage tables should be an array")
        .iter()
        .flat_map(|table| {
            let map_id = table["map_id"].as_str().expect("map id should be text");
            let table_index = table["table_index"]
                .as_u64()
                .expect("table index should be numeric");
            table["unsupported_rows"]
                .as_array()
                .expect("unsupported rows should be an array")
                .iter()
                .map(move |row| {
                    (
                        map_id,
                        table_index,
                        row["row_index"]
                            .as_u64()
                            .expect("row index should be numeric"),
                    )
                })
        })
        .collect::<std::collections::BTreeSet<_>>();
    for schedule in catalog.schedules() {
        for event in schedule.events() {
            for source in event.source_refs() {
                assert!(
                    !unsupported_source_rows.contains(&(
                        schedule.id(),
                        source.table_index as u64,
                        source.row_index as u64,
                    )),
                    "runtime event {} came from an unsupported source row",
                    event.id()
                );
            }
        }
    }
}
