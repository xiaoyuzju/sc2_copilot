use sc2_copilot_core::ScheduleCatalog;
use schedule_data::{KeiframeTimeRecord, compare_keiframe_times};

#[test]
fn difference_report_preserves_simultaneous_wiki_events() {
    let catalog = ScheduleCatalog::from_json(
        r#"
        {
          "schema_version": 1,
          "snapshot_batch": "test",
          "maps": [{
            "map_id": "oblivion-express",
            "display_name": "湮灭快车",
            "events": [
              {
                "map_id": "oblivion-express",
                "event_id": "first-a",
                "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
                "facts": [],
                "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
                "runtime_support": "automatic"
              },
              {
                "map_id": "oblivion-express",
                "event_id": "first-b",
                "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
                "facts": [],
                "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 2 }],
                "runtime_support": "automatic"
              },
              {
                "map_id": "oblivion-express",
                "event_id": "second",
                "trigger": { "kind": "at_game_time", "milliseconds": 120000 },
                "facts": [],
                "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 3 }],
                "runtime_support": "automatic"
              }
            ]
          }]
        }
        "#
        .as_bytes(),
    )
    .expect("fixture catalog should be valid");
    let records = vec![
        KeiframeTimeRecord::new("湮灭快车", 60),
        KeiframeTimeRecord::new("湮灭快车", 180),
    ];

    let report = compare_keiframe_times(&catalog, &records);
    let diff = report
        .configs
        .iter()
        .find(|diff| diff.keiframe_config == "湮灭快车")
        .expect("Oblivion Express mapping should be present");

    assert_eq!(diff.matching_times, vec![60]);
    assert_eq!(diff.wiki_only_times, vec![120]);
    assert_eq!(diff.keiframe_only_times, vec![180]);
    assert_eq!(diff.wiki_simultaneous_times, vec![60]);
}
