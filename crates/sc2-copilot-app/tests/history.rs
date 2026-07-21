use std::time::{SystemTime, UNIX_EPOCH};

use sc2_copilot_app::{
    AlertCard, AppController, AppSettings, NoopAlertPlayer, Sc2Observation, Sc2Poll, SessionHistory,
};
use sc2_copilot_core::{EventTiming, ScheduleCatalog};

const CATALOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/maps/catalog.json"
));

#[test]
fn session_history_records_decisions_and_alerts_without_poll_spam() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be valid")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sc2-copilot-history-test-{}-{suffix}",
        std::process::id()
    ));
    let path = directory.join("history.jsonl");
    let mut history = SessionHistory::new(path.clone()).expect("history should open");
    let catalog = ScheduleCatalog::from_json(CATALOG).expect("catalog should load");
    let mut controller =
        AppController::new(catalog, AppSettings::default(), Box::new(NoopAlertPlayer));

    controller.handle_poll(in_game(181_000));
    assert!(
        history
            .record(&controller, &[])
            .expect("first record should write")
    );

    controller.handle_poll(in_game(181_900));
    assert!(
        !history
            .record(&controller, &[])
            .expect("same second should deduplicate")
    );

    controller.select_variant(Some("layout-a".to_owned()));
    assert!(
        history
            .record(&controller, &[])
            .expect("variant change should write")
    );

    let alert = AlertCard {
        timing: EventTiming::Exact {
            milliseconds: 240_000,
        },
        event_ids: vec!["temple-example-event".to_owned()],
        event_labels: vec!["测试红点".to_owned()],
    };
    assert!(
        history
            .record(&controller, &[alert])
            .expect("alert should write")
    );

    let records = std::fs::read_to_string(&path).expect("history should be readable");
    let records = records
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("valid JSONL"))
        .collect::<Vec<_>>();

    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["state"], "in_game");
    assert_eq!(records[0]["detected_map_id"], "temple-of-the-past");
    assert_eq!(records[0]["selected_variant_id"], serde_json::Value::Null);
    assert_eq!(records[0]["upcoming_event_ids"], serde_json::json!([]));
    assert_eq!(records[1]["selected_variant_id"], "layout-a");
    assert!(
        records[1]["upcoming_event_ids"]
            .as_array()
            .is_some_and(|events| !events.is_empty())
    );
    assert_eq!(
        records[2]["alert_event_ids"],
        serde_json::json!([["temple-example-event"]])
    );

    controller.handle_poll(Sc2Poll {
        observation: Sc2Observation::InGame {
            session_id: "unknown-map-session".to_owned(),
            map_id: None,
            game_time_milliseconds: 12_345,
            player_count: 2,
        },
        diagnostic: None,
    });
    history
        .record(&controller, &[])
        .expect("unknown map state should write");
    controller.handle_poll(Sc2Poll {
        observation: Sc2Observation::Disconnected,
        diagnostic: None,
    });
    history
        .record(&controller, &[])
        .expect("disconnected state should write");

    drop(history);
    let records = std::fs::read_to_string(&path).expect("history should be readable");
    let records = records
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("valid JSONL"))
        .collect::<Vec<_>>();
    assert_eq!(records[3]["session_id"], "unknown-map-session");
    assert_eq!(records[3]["game_time_milliseconds"], 12_345);
    assert_eq!(records[3]["selected_variant_id"], serde_json::Value::Null);
    assert_eq!(records[3]["upcoming_event_ids"], serde_json::json!([]));
    assert_eq!(records[4]["state"], "disconnected");
    assert_eq!(records[4]["session_id"], serde_json::Value::Null);
    assert_eq!(records[4]["selected_variant_id"], serde_json::Value::Null);
    assert_eq!(records[4]["upcoming_event_ids"], serde_json::json!([]));

    std::fs::remove_file(path).expect("test history should be removable");
    std::fs::remove_dir(directory).expect("test directory should be removable");
}

fn in_game(game_time_milliseconds: u64) -> Sc2Poll {
    Sc2Poll {
        observation: Sc2Observation::InGame {
            session_id: "temple-session".to_owned(),
            map_id: Some("temple-of-the-past".to_owned()),
            game_time_milliseconds,
            player_count: 2,
        },
        diagnostic: None,
    }
}
