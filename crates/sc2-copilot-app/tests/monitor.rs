use sc2_copilot_app::{MonitorReducer, Sc2Observation, Sc2Poll};

#[test]
fn monitor_emits_sanitized_json_once_per_game_second_and_on_state_change() {
    let mut monitor = MonitorReducer::default();
    let first = monitor
        .observe(&in_game(1_100, None))
        .expect("first state should be emitted");
    assert_eq!(
        serde_json::to_value(first).expect("record should serialize"),
        serde_json::json!({
            "sequence": 1,
            "state": "in_game",
            "session_id": "session-a",
            "map_id": null,
            "game_time_milliseconds": 1_100,
            "player_count": 7,
            "diagnostic": null
        })
    );

    assert!(monitor.observe(&in_game(1_900, None)).is_none());
    assert!(monitor.observe(&in_game(2_050, None)).is_some());

    let identified = monitor
        .observe(&in_game(2_100, Some("oblivion-express")))
        .expect("map identification should emit immediately");
    assert_eq!(identified.map_id.as_deref(), Some("oblivion-express"));

    let disconnected = monitor
        .observe(&Sc2Poll {
            observation: Sc2Observation::Disconnected,
            diagnostic: Some("connection refused".to_owned()),
        })
        .expect("disconnect should emit immediately");
    assert_eq!(disconnected.state, "disconnected");
    assert_eq!(
        disconnected.diagnostic.as_deref(),
        Some("connection refused")
    );
}

fn in_game(game_time_milliseconds: u64, map_id: Option<&str>) -> Sc2Poll {
    Sc2Poll {
        observation: Sc2Observation::InGame {
            session_id: "session-a".to_owned(),
            map_id: map_id.map(str::to_owned),
            game_time_milliseconds,
            player_count: 7,
        },
        diagnostic: None,
    }
}
