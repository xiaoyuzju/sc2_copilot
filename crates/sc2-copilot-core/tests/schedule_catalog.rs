use sc2_copilot_core::{
    CatalogError, EventCategory, Fact, LocationSpec, RuntimeSupport, ScheduleCatalog, SourceRef,
    Trigger,
};

const CATALOG_JSON: &[u8] = include_bytes!("../../../data/maps/catalog.json");

#[test]
fn catalog_loads_oblivion_express_without_collapsing_simultaneous_events() {
    let catalog = ScheduleCatalog::from_json(CATALOG_JSON).expect("catalog should be valid");
    assert_eq!(catalog.map_count(), 1);

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

    assert_eq!(events_at_25_minutes, ["train-wave-8-1", "train-wave-8-2"]);
    let first_event = &schedule.events()[0];
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

    let first_train = &schedule.events()[1];
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
        .find(|event| event.id() == "train-wave-8-1")
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
