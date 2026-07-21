use serde::Serialize;

use crate::{Sc2Observation, Sc2Poll};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MonitorRecord {
    pub sequence: u64,
    pub state: String,
    pub session_id: Option<String>,
    pub map_id: Option<String>,
    pub game_time_milliseconds: Option<u64>,
    pub player_count: Option<usize>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorKey {
    state: &'static str,
    session_id: Option<String>,
    map_id: Option<String>,
    game_time_second: Option<u64>,
    player_count: Option<usize>,
    diagnostic: Option<String>,
}

#[derive(Debug, Default)]
pub struct MonitorReducer {
    sequence: u64,
    previous: Option<MonitorKey>,
}

impl MonitorReducer {
    pub fn observe(&mut self, poll: &Sc2Poll) -> Option<MonitorRecord> {
        let (key, game_time_milliseconds) = match &poll.observation {
            Sc2Observation::Disconnected => (
                MonitorKey {
                    state: "disconnected",
                    session_id: None,
                    map_id: None,
                    game_time_second: None,
                    player_count: None,
                    diagnostic: poll.diagnostic.clone(),
                },
                None,
            ),
            Sc2Observation::Menu => (
                MonitorKey {
                    state: "menu",
                    session_id: None,
                    map_id: None,
                    game_time_second: None,
                    player_count: None,
                    diagnostic: poll.diagnostic.clone(),
                },
                None,
            ),
            Sc2Observation::InGame {
                session_id,
                map_id,
                game_time_milliseconds,
                player_count,
            } => (
                MonitorKey {
                    state: "in_game",
                    session_id: Some(session_id.clone()),
                    map_id: map_id.clone(),
                    game_time_second: Some(game_time_milliseconds / 1_000),
                    player_count: Some(*player_count),
                    diagnostic: poll.diagnostic.clone(),
                },
                Some(*game_time_milliseconds),
            ),
        };
        if self.previous.as_ref() == Some(&key) {
            return None;
        }
        self.sequence += 1;
        let record = MonitorRecord {
            sequence: self.sequence,
            state: key.state.to_owned(),
            session_id: key.session_id.clone(),
            map_id: key.map_id.clone(),
            game_time_milliseconds,
            player_count: key.player_count,
            diagnostic: key.diagnostic.clone(),
        };
        self.previous = Some(key);
        Some(record)
    }
}
