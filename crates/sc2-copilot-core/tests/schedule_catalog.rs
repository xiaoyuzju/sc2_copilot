use sc2_copilot_core::{
    CatalogError, EventCategory, Fact, LocationSpec, RuntimeSupport, ScheduleCatalog, SourceRef,
    Trigger, UnsupportedReason,
};

const CATALOG_JSON: &[u8] = include_bytes!("../../../data/maps/catalog.json");

#[test]
fn catalog_loads_oblivion_express_without_collapsing_simultaneous_events() {
    let catalog = ScheduleCatalog::from_json(CATALOG_JSON).expect("catalog should be valid");
    assert_eq!(catalog.map_count(), 15);

    let schedule = catalog
        .schedule_for("oblivion-express", None)
        .expect("Oblivion Express should be present");

    assert_eq!(schedule.events().len(), 20);

    let events_at_25_minutes = schedule
        .events()
        .iter()
        .filter(|event| {
            event.trigger()
                == &Trigger::AtGameTime {
                    milliseconds: 1_500_000,
                }
        })
        .map(|event| event.id())
        .collect::<Vec<_>>();

    assert_eq!(
        events_at_25_minutes,
        ["oblivion-express-t0-r10-c1", "oblivion-express-t0-r11-c1"]
    );
    let first_event = schedule
        .events()
        .iter()
        .find(|event| event.id() == "oblivion-express-t3-r1-c1")
        .expect("first attack wave should exist");
    assert_eq!(first_event.map_id(), "oblivion-express");
    assert_eq!(first_event.variant_id(), None);
    assert!(first_event.facts().iter().any(|fact| matches!(
        fact,
        Fact::EventCategory {
            value: EventCategory::AttackWave
        }
    )));
    assert!(
        first_event
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::ScaleLevel { value: 1 }))
    );
    assert!(first_event.facts().iter().any(|fact| matches!(
        fact,
        Fact::Wave {
            number: 1,
            branch: None
        }
    )));

    let first_train = schedule
        .events()
        .iter()
        .find(|event| event.id() == "oblivion-express-t0-r1-c1")
        .expect("first train should exist");
    assert!(
        first_train
            .facts()
            .iter()
            .any(|fact| matches!(fact, Fact::Health { value: 4_200 }))
    );
    assert!(first_train.facts().iter().any(|fact| matches!(
        fact,
        Fact::UnitCount {
            unit: sc2_copilot_core::UnitKind::TrainCar,
            value: 7
        }
    )));

    let branched_train = schedule
        .events()
        .iter()
        .find(|event| event.id() == "oblivion-express-t0-r10-c1")
        .expect("branched train should exist");
    assert!(branched_train.facts().iter().any(|fact| matches!(
        fact,
        Fact::Wave {
            number: 8,
            branch: Some(1)
        }
    )));
    assert!(branched_train.facts().iter().any(|fact| matches!(
        fact,
        Fact::Location {
            value: LocationSpec::Any { options }
        } if options.len() == 2
            && options.iter().all(|option| option.weight_percent == Some(50))
    )));
    assert_eq!(first_event.runtime_support(), RuntimeSupport::Automatic);
    let _: &[SourceRef] = first_event.source_refs();
    assert!(
        schedule
            .events()
            .iter()
            .all(|event| !event.source_refs().is_empty())
    );
}

#[test]
fn catalog_rejects_blank_source_reference_fields() {
    let invalid_catalog = br#"
    {
      "schema_version": 1,
      "maps": [{
        "map_id": "test-map",
        "display_name": "Test Map",
        "events": [{
          "map_id": "test-map",
          "event_id": "orphan-event",
          "trigger": { "kind": "at_game_time", "milliseconds": 1000 },
          "facts": [],
          "source_refs": [{
            "source_url": " ",
            "snapshot_batch": "test",
            "snapshot_path": "fixture.json",
            "table_index": 0,
            "row_index": 1
          }],
          "runtime_support": "automatic"
        }]
      }]
    }
    "#;

    let error = ScheduleCatalog::from_json(invalid_catalog).expect_err("catalog must be rejected");

    assert!(matches!(
        error,
        CatalogError::InvalidSourceRef { map_id, event_id }
            if map_id == "test-map" && event_id == "orphan-event"
    ));
}

#[test]
fn catalog_rejects_duplicate_event_ids_within_a_map() {
    let invalid_catalog = br#"
    {
      "schema_version": 1,
      "maps": [{
        "map_id": "test-map",
        "display_name": "Test Map",
        "events": [
          {
            "map_id": "test-map",
            "event_id": "duplicate",
            "trigger": { "kind": "at_game_time", "milliseconds": 1000 },
            "facts": [],
            "source_refs": [{
              "source_url": "https://example.invalid/map",
              "snapshot_batch": "test",
              "snapshot_path": "fixture.json",
              "table_index": 0,
              "row_index": 1
            }],
            "runtime_support": "automatic"
          },
          {
            "map_id": "test-map",
            "event_id": "duplicate",
            "trigger": { "kind": "at_game_time", "milliseconds": 2000 },
            "facts": [],
            "source_refs": [{
              "source_url": "https://example.invalid/map",
              "snapshot_batch": "test",
              "snapshot_path": "fixture.json",
              "table_index": 0,
              "row_index": 2
            }],
            "runtime_support": "automatic"
          }
        ]
      }]
    }
    "#;

    let error = ScheduleCatalog::from_json(invalid_catalog).expect_err("catalog must be rejected");

    assert!(matches!(
        error,
        CatalogError::DuplicateEventId { map_id, event_id }
            if map_id == "test-map" && event_id == "duplicate"
    ));
}

#[test]
fn catalog_rejects_events_without_a_source_reference() {
    let invalid_catalog = br#"
    {
      "schema_version": 1,
      "maps": [{
        "map_id": "test-map",
        "display_name": "Test Map",
        "events": [{
          "map_id": "test-map",
          "event_id": "orphan-event",
          "trigger": { "kind": "at_game_time", "milliseconds": 1000 },
          "facts": [],
          "source_refs": [],
          "runtime_support": "automatic"
        }]
      }]
    }
    "#;

    let error = ScheduleCatalog::from_json(invalid_catalog).expect_err("catalog must be rejected");

    assert!(matches!(
        error,
        CatalogError::MissingSourceRef { map_id, event_id }
            if map_id == "test-map" && event_id == "orphan-event"
    ));
}

#[test]
fn catalog_rejects_an_event_owned_by_another_map() {
    let invalid_catalog = br#"
    {
      "schema_version": 1,
      "maps": [{
        "map_id": "test-map",
        "display_name": "Test Map",
        "events": [{
          "map_id": "another-map",
          "event_id": "misplaced-event",
          "trigger": { "kind": "at_game_time", "milliseconds": 1000 },
          "facts": [],
          "source_refs": [{
            "source_url": "https://example.invalid/map",
            "snapshot_batch": "test",
            "snapshot_path": "fixture.json",
            "table_index": 0,
            "row_index": 1
          }],
          "runtime_support": "automatic"
        }]
      }]
    }
    "#;

    let error = ScheduleCatalog::from_json(invalid_catalog).expect_err("catalog must be rejected");

    assert!(matches!(
        error,
        CatalogError::EventMapMismatch {
            schedule_map_id,
            event_map_id,
            event_id
        } if schedule_map_id == "test-map"
            && event_map_id == "another-map"
            && event_id == "misplaced-event"
    ));
}

#[test]
fn catalog_exposes_variants_stage_triggers_and_source_coverage() {
    let catalog = ScheduleCatalog::from_json(
        br#"
        {
          "schema_version": 1,
          "snapshot_batch": "test",
          "maps": [{
            "map_id": "night-map",
            "display_name": "Night Map",
            "variants": [{ "variant_id": "route-a", "display_name": "Route A" }],
            "events": [{
              "map_id": "night-map",
              "variant_id": "route-a",
              "event_id": "night-wave",
              "trigger": {
                "kind": "at_stage_elapsed",
                "stage_id": "night-1",
                "milliseconds": 30000
              },
              "facts": [],
              "source_refs": [{
                "source_url": "https://example.invalid/night-map",
                "snapshot_batch": "test",
                "snapshot_path": "night-map.json",
                "table_index": 2,
                "row_index": 1
              }],
              "runtime_support": "manual_context"
            }],
            "coverage": [{
              "source_url": "https://example.invalid/night-map",
              "snapshot_batch": "test",
              "snapshot_path": "night-map.json",
              "table_index": 2,
              "runtime_support": "manual_context",
              "unsupported_rows": [{
                "row_index": 3,
                "reason": "source_expression_unsupported"
              }]
            }]
          }]
        }
        "#,
    )
    .expect("extended catalog should be valid");

    assert_eq!(catalog.snapshot_batch(), "test");
    let schedule = catalog
        .schedule_for("night-map", Some("route-a"))
        .expect("known variant should resolve");
    assert_eq!(schedule.variants()[0].id(), "route-a");
    assert_eq!(schedule.coverage()[0].table_index(), 2);
    assert_eq!(
        schedule.coverage()[0].unsupported_rows()[0].reason(),
        UnsupportedReason::SourceExpressionUnsupported
    );
    assert_eq!(
        schedule.events()[0].trigger(),
        &Trigger::AtStageElapsed {
            stage_id: "night-1".to_owned(),
            milliseconds: 30_000
        }
    );
}

#[test]
fn catalog_rejects_a_time_window_with_reversed_bounds() {
    let json = br#"
    {
      "schema_version": 1,
      "maps": [{
        "map_id": "test-map",
        "display_name": "Test Map",
        "events": [{
          "map_id": "test-map",
          "event_id": "invalid-window",
          "trigger": {
            "kind": "at_game_time_window",
            "earliest_milliseconds": 2000,
            "latest_milliseconds": 1000
          },
          "facts": [],
          "source_refs": [{
            "source_url": "https://example.invalid/map",
            "snapshot_batch": "test",
            "snapshot_path": "fixture.json",
            "table_index": 0,
            "row_index": 1
          }],
          "runtime_support": "automatic"
        }]
      }]
    }
    "#;

    let error = ScheduleCatalog::from_json(json).expect_err("reversed window should fail");

    assert!(matches!(
        error,
        CatalogError::InvalidTimeWindow {
            map_id,
            event_id,
            earliest_milliseconds: 2_000,
            latest_milliseconds: 1_000,
        } if map_id == "test-map" && event_id == "invalid-window"
    ));
}
