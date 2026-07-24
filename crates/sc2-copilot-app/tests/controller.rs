use std::{cell::RefCell, rc::Rc};

use sc2_copilot_app::{
    AlertPlayer, AppController, AppSettings, NoopAlertPlayer, Sc2Observation, Sc2Poll,
    SettingsStore,
};
use sc2_copilot_core::{AlertBatch, EventTiming, ScheduleCatalog};
use sc2_copilot_vision::VisionUpdate;

const CATALOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/maps/catalog.json"
));

#[test]
fn controller_uses_manual_map_fallback_and_calls_player_once_per_batch() {
    let played = Rc::new(RefCell::new(Vec::new()));
    let player = RecordingPlayer {
        played: Rc::clone(&played),
        failure: None,
    };
    let mut controller = controller(Box::new(player));

    let first = controller.handle_poll(in_game("session-a", None, 5_000));
    assert!(first.new_alerts.is_empty());
    assert!(controller.view().session_id.is_none());

    controller.select_manual_map(Some("oblivion-express".to_owned()));
    let update = controller.handle_poll(in_game("session-a", None, 235_000));

    assert!(!update.new_alerts.is_empty());
    assert_eq!(played.borrow().len(), update.new_alerts.len());
    assert!(
        update
            .new_alerts
            .iter()
            .all(|card| card.event_ids.len() == card.event_labels.len())
    );

    controller.handle_poll(in_game("session-a", None, 236_000));
    assert_eq!(played.borrow().len(), update.new_alerts.len());
}

#[test]
fn playback_failure_is_diagnostic_and_does_not_repeat_the_alert() {
    let played = Rc::new(RefCell::new(Vec::new()));
    let player = RecordingPlayer {
        played: Rc::clone(&played),
        failure: Some("no device".to_owned()),
    };
    let mut controller = controller(Box::new(player));

    let first = controller.handle_poll(in_game("session-a", Some("oblivion-express"), 235_000));
    let second = controller.handle_poll(in_game("session-a", Some("oblivion-express"), 236_000));

    assert!(!first.new_alerts.is_empty());
    assert!(second.new_alerts.is_empty());
    assert!(
        controller
            .diagnostics()
            .iter()
            .any(|line| line == "提醒播放失败：no device")
    );
}

#[test]
fn settings_store_round_trips_persistent_settings() {
    let directory =
        std::env::temp_dir().join(format!("sc2-copilot-settings-test-{}", std::process::id()));
    let store = SettingsStore::new(directory.join("settings.json"));
    let settings = AppSettings {
        lead_time_seconds: 18,
        overlay_position: [100.5, 220.0],
        overlay_size: [560.0, 360.0],
        hotkey: Some("Control+Shift+KeyO".to_owned()),
    };

    store.save(&settings).expect("settings should save");
    assert_eq!(store.load().expect("settings should load"), settings);

    std::fs::remove_file(store.path()).expect("test settings should be removable");
    std::fs::remove_dir(directory).expect("test directory should be removable");
}

#[test]
fn noop_player_accepts_a_batch_without_choosing_a_sound_provider() {
    let mut player = NoopAlertPlayer;
    let batch = AlertBatch {
        timing: EventTiming::Exact {
            milliseconds: 10_000,
        },
        event_ids: vec!["example".to_owned()],
    };

    player.play(&batch).expect("no-op delivery should succeed");
    assert_eq!(player.status(), "未配置（提醒接口已保留）");
}

#[test]
fn controller_applies_manual_mutator_context_only_for_the_current_session() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-1", Some("oblivion-express"), 0));

    let map = controller
        .map("oblivion-express")
        .expect("known map should be described");
    assert_eq!(map.mutators.len(), 1);
    assert_eq!(map.mutators[0].id, "polarity");
    let event_id = "oblivion-express-t0-r1-c1";
    assert!(!controller.event_label(event_id).contains("可造成伤害玩家"));

    controller.set_mutator_active("polarity".to_owned(), true);
    assert!(controller.event_label(event_id).contains("可造成伤害玩家"));

    controller.handle_poll(Sc2Poll {
        observation: Sc2Observation::Menu,
        diagnostic: None,
    });
    controller.handle_poll(in_game("session-2", Some("oblivion-express"), 0));
    assert!(!controller.event_label(event_id).contains("可造成伤害玩家"));
}

#[test]
fn controller_defaults_temple_of_the_past_to_schedule_b_per_session() {
    let mut controller = controller(Box::new(NoopAlertPlayer));

    controller.handle_poll(in_game(
        "temple-session-1",
        Some("temple-of-the-past"),
        1_000,
    ));
    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-b"));
    assert_eq!(controller.variant_selection_source_label(), "目录默认");
    assert!(!controller.view().upcoming_events.is_empty());

    controller.select_variant(Some("layout-b".to_owned()));
    assert_eq!(controller.variant_selection_source_label(), "用户手动");

    controller.select_variant(Some("layout-a".to_owned()));
    assert_eq!(controller.variant_selection_source_label(), "用户手动");
    controller.handle_poll(in_game(
        "temple-session-1",
        Some("temple-of-the-past"),
        2_000,
    ));
    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-a"));

    controller.handle_poll(in_game(
        "temple-session-2",
        Some("temple-of-the-past"),
        1_000,
    ));
    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-b"));
}

#[test]
fn controller_applies_a_stable_visual_variant_for_the_current_session() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-1", Some("void-rifts"), 1_000));

    controller.handle_vision(VisionUpdate::map_variant(
        "session-1",
        "void-rifts",
        "layout-a",
    ));

    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-a"));
    assert_eq!(controller.variant_selection_source_label(), "视觉识别");
}

#[test]
fn a_manual_variant_is_not_overwritten_by_visual_evidence() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-1", Some("void-rifts"), 1_000));
    controller.select_variant(Some("layout-b".to_owned()));

    controller.handle_vision(VisionUpdate::map_variant(
        "session-1",
        "void-rifts",
        "layout-a",
    ));

    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-b"));
}

#[test]
fn a_manual_variant_remains_authoritative_after_reconnecting_to_the_same_session() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-1", Some("void-rifts"), 1_000));
    controller.select_variant(Some("layout-b".to_owned()));
    controller.handle_poll(Sc2Poll {
        observation: Sc2Observation::Disconnected,
        diagnostic: None,
    });
    controller.handle_poll(in_game("session-1", Some("void-rifts"), 2_000));

    controller.handle_vision(VisionUpdate::map_variant(
        "session-1",
        "void-rifts",
        "layout-a",
    ));

    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-b"));
}

#[test]
fn a_manual_variant_from_another_map_does_not_block_visual_evidence() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-1", Some("temple-of-the-past"), 1_000));
    controller.select_variant(Some("layout-b".to_owned()));
    controller.handle_poll(in_game("session-1", Some("void-rifts"), 2_000));

    controller.handle_vision(VisionUpdate::map_variant(
        "session-1",
        "void-rifts",
        "layout-a",
    ));

    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-a"));
}

#[test]
fn a_manual_variant_source_resets_when_the_fallback_map_changes() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-1", None, 1_000));
    controller.select_manual_map(Some("temple-of-the-past".to_owned()));
    controller.select_variant(Some("layout-b".to_owned()));
    controller.select_manual_map(Some("void-rifts".to_owned()));

    controller.handle_vision(VisionUpdate::map_variant(
        "session-1",
        "void-rifts",
        "layout-a",
    ));

    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-a"));
}

#[test]
fn stale_or_invalid_visual_variants_are_ignored() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game("session-2", Some("void-rifts"), 1_000));

    for update in [
        VisionUpdate::map_variant("session-1", "void-rifts", "layout-a"),
        VisionUpdate::map_variant("session-2", "temple-of-the-past", "layout-a"),
        VisionUpdate::map_variant("session-2", "void-rifts", "unknown-layout"),
    ] {
        controller.handle_vision(update);
    }

    assert_eq!(controller.view().variant_id, None);
}

#[test]
fn visual_evidence_replaces_defaults_after_manual_state_resets_for_a_new_session() {
    let mut controller = controller(Box::new(NoopAlertPlayer));
    controller.handle_poll(in_game(
        "temple-session-1",
        Some("temple-of-the-past"),
        1_000,
    ));
    controller.handle_vision(VisionUpdate::map_variant(
        "temple-session-1",
        "temple-of-the-past",
        "layout-a",
    ));
    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-a"));

    controller.select_variant(Some("layout-b".to_owned()));
    controller.handle_poll(in_game(
        "temple-session-2",
        Some("temple-of-the-past"),
        1_000,
    ));
    controller.handle_vision(VisionUpdate::map_variant(
        "temple-session-2",
        "temple-of-the-past",
        "layout-a",
    ));

    assert_eq!(controller.view().variant_id.as_deref(), Some("layout-a"));
}

fn controller(player: Box<dyn AlertPlayer>) -> AppController {
    let catalog = ScheduleCatalog::from_json(CATALOG).expect("embedded catalog should parse");
    AppController::new(catalog, AppSettings::default(), player)
}

fn in_game(session_id: &str, map_id: Option<&str>, time: u64) -> Sc2Poll {
    Sc2Poll {
        observation: Sc2Observation::InGame {
            session_id: session_id.to_owned(),
            map_id: map_id.map(str::to_owned),
            game_time_milliseconds: time,
            player_count: 2,
        },
        diagnostic: None,
    }
}

struct RecordingPlayer {
    played: Rc<RefCell<Vec<AlertBatch>>>,
    failure: Option<String>,
}

impl AlertPlayer for RecordingPlayer {
    fn play(&mut self, batch: &AlertBatch) -> Result<(), String> {
        self.played.borrow_mut().push(batch.clone());
        self.failure.clone().map_or(Ok(()), Err)
    }

    fn status(&self) -> &str {
        "test player"
    }
}
