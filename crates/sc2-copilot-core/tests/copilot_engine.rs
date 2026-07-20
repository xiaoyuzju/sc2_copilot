use sc2_copilot_core::{
    CopilotEngine, EngineInput, EngineSettings, GameObservation, ScheduleCatalog, UserCommand,
};

#[test]
fn event_notifies_once_when_it_enters_the_default_lead_window() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "wave-1",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    assert!(observe(&mut engine, 0).alert_batches.is_empty());
    assert!(observe(&mut engine, 29_999).alert_batches.is_empty());

    let update = observe(&mut engine, 30_000);
    assert_eq!(update.alert_batches.len(), 1);
    assert_eq!(update.alert_batches[0].event_ids, ["wave-1"]);

    assert!(observe(&mut engine, 60_000).alert_batches.is_empty());
    assert!(observe(&mut engine, 60_001).alert_batches.is_empty());
}

#[test]
fn simultaneous_events_share_one_batch_but_adjacent_times_do_not_merge() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "same-b",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        },
        {
          "map_id": "test-map",
          "event_id": "same-a",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 2 }],
          "runtime_support": "automatic"
        },
        {
          "map_id": "test-map",
          "event_id": "adjacent",
          "trigger": { "kind": "at_game_time", "milliseconds": 61000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 3 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    let simultaneous = observe(&mut engine, 30_000);
    assert_eq!(simultaneous.alert_batches.len(), 1);
    assert_eq!(
        simultaneous.alert_batches[0].event_ids,
        ["same-a", "same-b"]
    );

    let adjacent = observe(&mut engine, 31_000);
    assert_eq!(adjacent.alert_batches.len(), 1);
    assert_eq!(adjacent.alert_batches[0].event_ids, ["adjacent"]);
}

#[test]
fn late_start_marks_past_events_missed_and_alerts_future_events_in_window() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "past",
          "trigger": { "kind": "at_game_time", "milliseconds": 40000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        },
        {
          "map_id": "test-map",
          "event_id": "future",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 2 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    let update = observe(&mut engine, 50_000);

    assert_eq!(update.newly_missed_event_ids, ["past"]);
    assert_eq!(update.alert_batches[0].event_ids, ["future"]);
}

#[test]
fn changed_global_lead_time_applies_to_pending_events() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "wave-1",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());
    engine.apply(EngineInput::SettingsChanged(EngineSettings {
        lead_time_milliseconds: 10_000,
    }));

    assert!(observe(&mut engine, 49_999).alert_batches.is_empty());
    assert_eq!(
        observe(&mut engine, 50_000).alert_batches[0].event_ids,
        ["wave-1"]
    );
}

#[test]
fn reconnect_preserves_deduplication_but_a_new_session_resets_it() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "wave-1",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    assert_eq!(
        observe_session(&mut engine, "session-1", 30_000).alert_batches[0].event_ids,
        ["wave-1"]
    );
    engine.apply(EngineInput::Observation(GameObservation::Disconnected));
    assert!(
        observe_session(&mut engine, "session-1", 31_000)
            .alert_batches
            .is_empty()
    );
    assert_eq!(
        observe_session(&mut engine, "session-2", 30_000).alert_batches[0].event_ids,
        ["wave-1"]
    );
}

#[test]
fn game_time_reset_clears_session_scoped_delivery_state() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "wave-1",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    assert_eq!(observe(&mut engine, 30_000).alert_batches.len(), 1);
    observe(&mut engine, 60_000);
    observe(&mut engine, 0);

    assert_eq!(observe(&mut engine, 30_000).alert_batches.len(), 1);
}

#[test]
fn backward_time_correction_within_a_session_does_not_replay_alerts() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "wave-1",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    assert_eq!(observe(&mut engine, 30_000).alert_batches.len(), 1);
    observe(&mut engine, 50_000);
    observe(&mut engine, 20_000);

    assert!(observe(&mut engine, 30_000).alert_batches.is_empty());
}

#[test]
fn manually_selected_variant_enables_only_its_events() {
    let catalog = catalog_with_map_body(
        r#"
        "variants": [
          { "variant_id": "route-a", "display_name": "Route A" },
          { "variant_id": "route-b", "display_name": "Route B" }
        ],
        "events": [
          {
            "map_id": "test-map",
            "variant_id": "route-a",
            "event_id": "route-a-wave",
            "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
            "facts": [],
            "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
            "runtime_support": "manual_context"
          },
          {
            "map_id": "test-map",
            "variant_id": "route-b",
            "event_id": "route-b-wave",
            "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
            "facts": [],
            "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 1, "row_index": 1 }],
            "runtime_support": "manual_context"
          }
        ]
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());
    assert!(observe(&mut engine, 30_000).alert_batches.is_empty());

    let selected = engine.apply(EngineInput::Command(UserCommand::SelectVariant {
        variant_id: Some("route-a".to_owned()),
    }));
    assert_eq!(selected.alert_batches[0].event_ids, ["route-a-wave"]);
    assert_eq!(selected.view.variant_id.as_deref(), Some("route-a"));

    let changed = engine.apply(EngineInput::Command(UserCommand::SelectVariant {
        variant_id: Some("route-b".to_owned()),
    }));
    assert_eq!(changed.alert_batches[0].event_ids, ["route-b-wave"]);

    let selected_again = engine.apply(EngineInput::Command(UserCommand::SelectVariant {
        variant_id: Some("route-a".to_owned()),
    }));
    assert!(selected_again.alert_batches.is_empty());
}

#[test]
fn manual_stage_anchor_schedules_relative_events_for_the_current_session() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "night-wave",
          "trigger": { "kind": "at_stage_elapsed", "stage_id": "night-1", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "manual_context"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());
    observe(&mut engine, 100_000);

    let anchored = engine.apply(EngineInput::Command(UserCommand::SetStageAnchor {
        stage_id: "night-1".to_owned(),
    }));
    assert_eq!(anchored.view.stage_anchors[0].stage_id, "night-1");
    assert_eq!(
        anchored.view.stage_anchors[0].game_time_milliseconds,
        100_000
    );
    assert!(observe(&mut engine, 129_999).alert_batches.is_empty());
    assert_eq!(
        observe(&mut engine, 130_000).alert_batches[0].event_ids,
        ["night-wave"]
    );

    engine.apply(EngineInput::Observation(GameObservation::Menu));
    let new_session = observe_session(&mut engine, "session-2", 100_000);
    assert!(new_session.view.stage_anchors.is_empty());
}

#[test]
fn replacing_or_clearing_an_anchor_recomputes_only_pending_events() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "night-wave",
          "trigger": { "kind": "at_stage_elapsed", "stage_id": "night-1", "milliseconds": 100000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "manual_context"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());
    observe(&mut engine, 100_000);
    engine.apply(EngineInput::Command(UserCommand::SetStageAnchor {
        stage_id: "night-1".to_owned(),
    }));
    observe(&mut engine, 140_000);
    engine.apply(EngineInput::Command(UserCommand::SetStageAnchor {
        stage_id: "night-1".to_owned(),
    }));

    assert!(observe(&mut engine, 209_999).alert_batches.is_empty());
    assert_eq!(
        observe(&mut engine, 210_000).alert_batches[0].event_ids,
        ["night-wave"]
    );

    engine.apply(EngineInput::Command(UserCommand::SetStageAnchor {
        stage_id: "night-1".to_owned(),
    }));
    assert!(observe(&mut engine, 280_000).alert_batches.is_empty());

    engine.apply(EngineInput::Command(UserCommand::ClearStageAnchor {
        stage_id: "night-1".to_owned(),
    }));
    assert!(observe(&mut engine, 400_000).alert_batches.is_empty());
}

#[test]
fn overlay_view_keeps_simultaneous_upcoming_events_separate_after_notification() {
    let catalog = catalog_with_events(
        r#"
        {
          "map_id": "test-map",
          "event_id": "same-b",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 1 }],
          "runtime_support": "automatic"
        },
        {
          "map_id": "test-map",
          "event_id": "same-a",
          "trigger": { "kind": "at_game_time", "milliseconds": 60000 },
          "facts": [],
          "source_refs": [{ "source_url": "https://example.invalid", "snapshot_batch": "test", "snapshot_path": "test.json", "table_index": 0, "row_index": 2 }],
          "runtime_support": "automatic"
        }
        "#,
    );
    let mut engine = CopilotEngine::new(catalog, EngineSettings::default());

    let update = observe(&mut engine, 30_000);

    assert_eq!(update.alert_batches.len(), 1);
    assert_eq!(update.view.upcoming_events.len(), 2);
    assert_eq!(update.view.upcoming_events[0].event_id, "same-a");
    assert_eq!(
        update.view.upcoming_events[0].remaining_milliseconds,
        30_000
    );
    assert_eq!(update.view.upcoming_events[1].event_id, "same-b");
}

fn observe(
    engine: &mut CopilotEngine,
    game_time_milliseconds: u64,
) -> sc2_copilot_core::EngineUpdate {
    observe_session(engine, "session-1", game_time_milliseconds)
}

fn observe_session(
    engine: &mut CopilotEngine,
    session_id: &str,
    game_time_milliseconds: u64,
) -> sc2_copilot_core::EngineUpdate {
    engine.apply(EngineInput::Observation(GameObservation::InGame {
        session_id: session_id.to_owned(),
        map_id: "test-map".to_owned(),
        game_time_milliseconds,
    }))
}

fn catalog_with_events(events: &str) -> ScheduleCatalog {
    catalog_with_map_body(&format!(r#""events": [{events}]"#))
}

fn catalog_with_map_body(map_body: &str) -> ScheduleCatalog {
    let json = format!(
        r#"
        {{
          "schema_version": 1,
          "snapshot_batch": "test",
          "maps": [{{
            "map_id": "test-map",
            "display_name": "Test Map",
            {map_body}
          }}]
        }}
        "#
    );
    ScheduleCatalog::from_json(json.as_bytes()).expect("fixture catalog should be valid")
}
