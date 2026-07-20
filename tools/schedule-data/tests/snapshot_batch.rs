use std::path::Path;

use schedule_data::validate_snapshot_batch;

#[test]
fn validates_the_versioned_huiji_snapshot_batch() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let stats = validate_snapshot_batch(&workspace_root.join("data/sources/huiji/2026-07-21"))
        .expect("checked-in snapshot batch should be valid");

    assert_eq!(stats.map_count, 15);
    assert_eq!(stats.table_count, 149);
    assert_eq!(stats.row_count, 1_609);
    assert_eq!(stats.merged_cell_count, 558);
}
