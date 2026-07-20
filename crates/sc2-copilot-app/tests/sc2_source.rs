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
    assert_eq!(player_count, 9);

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
