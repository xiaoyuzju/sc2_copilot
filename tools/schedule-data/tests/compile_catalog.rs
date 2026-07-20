use std::path::Path;

use sc2_copilot_core::{RuntimeSupport, ScheduleCatalog, Trigger, UnsupportedReason};
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

    let catalog = ScheduleCatalog::from_json(&output.catalog_json)
        .expect("compiler output should satisfy the runtime schema");
    assert_eq!(catalog.map_count(), 15);
    assert_eq!(catalog.snapshot_batch(), "2026-07-21");

    let rifts = catalog
        .schedule_for("void-rifts", Some("layout-a"))
        .expect("manual rift layout should exist");
    assert_eq!(rifts.variants().len(), 2);

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
}
