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

#[test]
fn difference_report_exposes_precision_structure_attribute_and_vocabulary_checks() {
    let catalog = ScheduleCatalog::from_json(
        r#"
        {
          "schema_version": 1,
          "snapshot_batch": "test",
          "maps": [{
            "map_id": "oblivion-express",
            "display_name": "湮灭快车",
            "events": [{
              "map_id": "oblivion-express",
              "event_id": "wave",
              "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
              "facts": [
                { "kind": "event_category", "value": "attack_wave" },
                { "kind": "wave", "number": 1 },
                { "kind": "health", "value": 1000 },
                { "kind": "route", "value": "右侧" }
              ],
              "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
              "runtime_support": "automatic"
            }]
          }]
        }
        "#
        .as_bytes(),
    )
    .expect("fixture catalog should be valid");
    let records = vec![KeiframeTimeRecord::with_comparison_fields(
        "湮灭快车",
        "01:01",
        60,
        Some("第1波 右侧"),
        Some("生命1000"),
    )];

    let report = compare_keiframe_times(&catalog, &records);
    let diff = report
        .configs
        .iter()
        .find(|diff| diff.keiframe_config == "湮灭快车")
        .expect("Oblivion Express mapping should be present");

    assert_eq!(diff.keiframe_time_label_mismatch_count, 1);
    assert_eq!(diff.wiki_structured_fact_count, 4);
    assert_eq!(diff.wiki_multi_fact_event_count, 1);
    assert_eq!(diff.keiframe_text_attribute_record_count, 1);
    assert_eq!(diff.keiframe_compound_record_count, 1);
    assert_eq!(diff.numeric_candidate_review_times, vec![60]);
    assert!(diff.shared_vocabulary_token_count > 0);
    assert!(diff.keiframe_only_vocabulary_token_count > 0);

    let serialized = serde_json::to_string(&report).expect("report should serialize");
    assert!(!serialized.contains("第1波 右侧"));
    assert!(!serialized.contains("生命1000"));
}
