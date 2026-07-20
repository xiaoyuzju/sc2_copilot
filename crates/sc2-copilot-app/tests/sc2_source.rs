use sc2_copilot_app::{
    LatestSc2Poll, Sc2EndpointClient, Sc2EndpointError, Sc2Normalizer, Sc2Observation, Sc2Poll,
    Sc2StateSource,
};

const MENU_GAME: &[u8] = include_bytes!("fixtures/sc2/menu-game.json");
const MENU_UI: &[u8] = include_bytes!("fixtures/sc2/menu-ui.json");
const GAME: &[u8] = include_bytes!("fixtures/sc2/oblivion-game.json");
const GAME_LATER: &[u8] = include_bytes!("fixtures/sc2/oblivion-game-later.json");
const IN_GAME_UI: &[u8] = include_bytes!("fixtures/sc2/in-game-ui.json");

#[test]
fn normalizes_menu_and_in_game_payloads_into_stable_sessions() {
    let mut normalizer = Sc2Normalizer::default();
    assert_eq!(
        normalizer
            .normalize(MENU_GAME, MENU_UI)
            .expect("menu fixture should parse"),
        Sc2Observation::Menu
    );

    let first = normalizer
        .normalize(GAME, IN_GAME_UI)
        .expect("game fixture should parse");
    let Sc2Observation::InGame {
        session_id,
        map_id,
        game_time_milliseconds,
        player_count,
    } = first
    else {
        panic!("expected in-game observation");
    };
    assert_eq!(map_id.as_deref(), Some("oblivion-express"));
    assert_eq!(game_time_milliseconds, 42_500);
    assert_eq!(player_count, 7);

    let later = normalizer
        .normalize(GAME_LATER, IN_GAME_UI)
        .expect("later game fixture should parse");
    assert!(matches!(
        later,
        Sc2Observation::InGame {
            session_id: later_session,
            game_time_milliseconds: 45_250,
            ..
        } if later_session == session_id
    ));

    normalizer
        .normalize(MENU_GAME, MENU_UI)
        .expect("menu fixture should parse");
    let next = normalizer
        .normalize(GAME, IN_GAME_UI)
        .expect("new game fixture should parse");
    assert!(matches!(
        next,
        Sc2Observation::InGame { session_id: next_session, .. }
            if next_session != session_id
    ));
}

#[test]
fn time_reset_near_game_start_creates_a_new_session_without_a_menu_sample() {
    let mut normalizer = Sc2Normalizer::default();
    let first = normalizer
        .normalize(GAME_LATER, IN_GAME_UI)
        .expect("game fixture should parse");
    let reset_payload = String::from_utf8(GAME.to_vec())
        .expect("fixture is UTF-8")
        .replace("42.5", "1.0");
    let reset = normalizer
        .normalize(reset_payload.as_bytes(), IN_GAME_UI)
        .expect("reset fixture should parse");

    let (
        Sc2Observation::InGame {
            session_id: first, ..
        },
        Sc2Observation::InGame {
            session_id: reset, ..
        },
    ) = (first, reset)
    else {
        panic!("expected in-game observations");
    };
    assert_ne!(reset, first);
}

#[test]
fn identifies_all_fifteen_maps_from_positional_player_signatures() {
    let cases = [
        (
            "chain-of-ascension",
            9,
            [(6, "吉娜拉"), (8, "斯雷恩元素生物")],
        ),
        (
            "cradle-of-death",
            6,
            [(4, "埃蒙的特战队"), (5, "埃蒙的特战队")],
        ),
        ("dead-of-night", 7, [(4, "感染体"), (5, "感应塔")]),
        ("lock-and-load", 4, [(2, "埃蒙的部队"), (3, "埃蒙的部队")]),
        ("malwarfare", 6, [(2, "净化者全息投影"), (4, "麦加利斯")]),
        (
            "miner-evacuation",
            11,
            [(7, "凯莫瑞安矿工"), (9, "凯莫瑞安矿工")],
        ),
        ("mist-opportunities", 6, [(4, "地嗪"), (5, "艾贡·斯台特曼")]),
        (
            "oblivion-express",
            7,
            [(5, "埃蒙的部队"), (6, "埃蒙的部队")],
        ),
        ("part-and-parcel", 6, [(4, "巴利俄斯"), (5, "莫比斯列车")]),
        ("rifts-to-korhal", 6, [(4, "虚空碎片"), (5, "海盗")]),
        ("scythe-of-amon", 8, [(6, "埃蒙的部队"), (7, "待救者")]),
        ("temple-of-the-past", 9, [(6, "神庙"), (8, "岩石")]),
        ("the-vermillion-problem", 8, [(6, "熔岩蜥蜴"), (7, "平民")]),
        ("void-launch", 7, [(5, "时空航道"), (6, "科研队伍")]),
        ("void-rifts", 6, [(4, "埃蒙的部队"), (5, "重锤军士的部队")]),
    ];

    for (map_id, player_count, checks) in cases {
        let mut players = (0..player_count)
            .map(|index| serde_json::json!({ "name": format!("占位 {index}") }))
            .collect::<Vec<_>>();
        for (index, name) in checks {
            players[index] = serde_json::json!({ "name": name });
        }
        let payload = serde_json::to_vec(&serde_json::json!({
            "displayTime": 15.0,
            "players": players
        }))
        .expect("test payload should serialize");
        let mut normalizer = Sc2Normalizer::default();
        let observation = normalizer
            .normalize(&payload, IN_GAME_UI)
            .expect("test payload should normalize");
        assert!(matches!(
            observation,
            Sc2Observation::InGame { map_id: Some(found), .. } if found == map_id
        ));
    }
}

#[test]
fn state_source_reads_both_local_endpoints_and_latest_slot_drops_stale_samples() {
    let client = FixtureClient::new(vec![GAME.to_vec(), IN_GAME_UI.to_vec()]);
    let mut source = Sc2StateSource::new(client);
    let poll = source.poll();
    assert!(matches!(
        poll.observation,
        Sc2Observation::InGame {
            game_time_milliseconds: 42_500,
            ..
        }
    ));
    assert!(poll.diagnostic.is_none());

    let latest = LatestSc2Poll::default();
    latest.publish(Sc2Poll {
        observation: Sc2Observation::Menu,
        diagnostic: None,
    });
    latest.publish(poll.clone());
    assert_eq!(latest.take(), Some(poll));
    assert_eq!(latest.take(), None);
}

#[test]
fn endpoint_failure_becomes_a_disconnected_diagnostic() {
    let client = FixtureClient::new(Vec::new());
    let mut source = Sc2StateSource::new(client);
    let poll = source.poll();

    assert_eq!(poll.observation, Sc2Observation::Disconnected);
    assert_eq!(
        poll.diagnostic.as_deref(),
        Some("SC2 6119 transport error: fixture exhausted")
    );
}

struct FixtureClient {
    responses: std::collections::VecDeque<Vec<u8>>,
}

impl FixtureClient {
    fn new(responses: Vec<Vec<u8>>) -> Self {
        Self {
            responses: responses.into(),
        }
    }
}

impl Sc2EndpointClient for FixtureClient {
    fn get(&mut self, _path: &str) -> Result<Vec<u8>, Sc2EndpointError> {
        self.responses
            .pop_front()
            .ok_or_else(|| Sc2EndpointError::Transport("fixture exhausted".to_owned()))
    }
}
