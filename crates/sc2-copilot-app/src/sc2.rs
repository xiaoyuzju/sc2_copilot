use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sc2Observation {
    Disconnected,
    Menu,
    InGame {
        session_id: String,
        map_id: Option<String>,
        game_time_milliseconds: u64,
        player_count: usize,
    },
}

pub trait Sc2EndpointClient {
    fn get(&mut self, path: &str) -> Result<Vec<u8>, Sc2EndpointError>;
}

const SC2_API_BASE_URL: &str = "http://127.0.0.1:6119";

#[derive(Debug, Clone)]
pub struct LocalSc2HttpClient {
    client: reqwest::blocking::Client,
}

impl LocalSc2HttpClient {
    pub fn new(timeout: Duration) -> Result<Self, Sc2EndpointError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| Sc2EndpointError::ClientBuild(error.to_string()))?;
        Ok(Self { client })
    }
}

impl Sc2EndpointClient for LocalSc2HttpClient {
    fn get(&mut self, path: &str) -> Result<Vec<u8>, Sc2EndpointError> {
        let response = self
            .client
            .get(format!("{SC2_API_BASE_URL}{path}"))
            .send()
            .map_err(|error| Sc2EndpointError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Sc2EndpointError::HttpStatus(status.as_u16()));
        }
        response
            .bytes()
            .map(|body| body.to_vec())
            .map_err(|error| Sc2EndpointError::Transport(error.to_string()))
    }
}

#[derive(Debug)]
pub struct Sc2StateSource<C> {
    client: C,
    normalizer: Sc2Normalizer,
}

impl<C: Sc2EndpointClient> Sc2StateSource<C> {
    pub fn new(client: C) -> Self {
        Self {
            client,
            normalizer: Sc2Normalizer::default(),
        }
    }

    pub fn poll(&mut self) -> Sc2Poll {
        let payloads = self
            .client
            .get("/game/")
            .and_then(|game| self.client.get("/ui/").map(|ui| (game, ui)));
        match payloads {
            Ok((game, ui)) => match self.normalizer.normalize(&game, &ui) {
                Ok(observation) => Sc2Poll {
                    observation,
                    diagnostic: None,
                },
                Err(error) => Sc2Poll {
                    observation: self.normalizer.disconnected(),
                    diagnostic: Some(error.to_string()),
                },
            },
            Err(error) => Sc2Poll {
                observation: self.normalizer.disconnected(),
                diagnostic: Some(error.to_string()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sc2Poll {
    pub observation: Sc2Observation,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LatestSc2Poll {
    inner: Arc<Mutex<Option<Sc2Poll>>>,
}

impl LatestSc2Poll {
    pub fn publish(&self, poll: Sc2Poll) {
        *self.inner.lock().unwrap_or_else(|error| error.into_inner()) = Some(poll);
    }

    pub fn take(&self) -> Option<Sc2Poll> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

#[derive(Debug)]
pub struct Sc2PollingHandle {
    latest: LatestSc2Poll,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Sc2PollingHandle {
    pub fn spawn<C>(mut source: Sc2StateSource<C>, interval: Duration) -> Self
    where
        C: Sc2EndpointClient + Send + 'static,
    {
        let latest = LatestSc2Poll::default();
        let producer = latest.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                producer.publish(source.poll());
                thread::sleep(interval);
            }
        });
        Self {
            latest,
            stop,
            worker: Some(worker),
        }
    }

    pub fn take_latest(&self) -> Option<Sc2Poll> {
        self.latest.take()
    }
}

impl Drop for Sc2PollingHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug, Default)]
pub struct Sc2Normalizer {
    session_sequence: u64,
    current_fingerprint: Option<u64>,
    last_game_time_milliseconds: Option<u64>,
    in_game: bool,
}

impl Sc2Normalizer {
    pub fn normalize(
        &mut self,
        game_payload: &[u8],
        ui_payload: &[u8],
    ) -> Result<Sc2Observation, Sc2NormalizeError> {
        let game: RawGame = serde_json::from_slice(game_payload)?;
        let ui: RawUi = serde_json::from_slice(ui_payload)?;
        let ui_is_in_game = ui
            .active_screens
            .iter()
            .any(|screen| screen.to_ascii_lowercase().contains("game"));
        if game.players.is_empty() || (!ui_is_in_game && game.display_time.unwrap_or(0.0) <= 0.0) {
            self.in_game = false;
            self.current_fingerprint = None;
            self.last_game_time_milliseconds = None;
            return Ok(Sc2Observation::Menu);
        }

        let display_time = game
            .display_time
            .filter(|time| time.is_finite() && *time >= 0.0)
            .ok_or(Sc2NormalizeError::InvalidDisplayTime)?;
        let game_time_milliseconds = (display_time * 1_000.0).round() as u64;
        let fingerprint = player_fingerprint(&game.players);
        let restarted_without_menu = self
            .last_game_time_milliseconds
            .is_some_and(|previous| previous >= 30_000 && game_time_milliseconds <= 5_000);
        if !self.in_game || self.current_fingerprint != Some(fingerprint) || restarted_without_menu
        {
            self.session_sequence += 1;
        }
        self.in_game = true;
        self.current_fingerprint = Some(fingerprint);
        self.last_game_time_milliseconds = Some(game_time_milliseconds);

        Ok(Sc2Observation::InGame {
            session_id: format!("sc2-session-{}", self.session_sequence),
            map_id: identify_map(&game.players).map(str::to_owned),
            game_time_milliseconds,
            player_count: game.players.len(),
        })
    }

    pub fn disconnected(&self) -> Sc2Observation {
        Sc2Observation::Disconnected
    }
}

fn player_fingerprint(players: &[RawPlayer]) -> u64 {
    let mut names = players
        .iter()
        .map(|player| player.name.trim().to_lowercase())
        .collect::<Vec<_>>();
    names.sort();
    let mut hasher = DefaultHasher::new();
    names.hash(&mut hasher);
    hasher.finish()
}

fn identify_map(players: &[RawPlayer]) -> Option<&'static str> {
    let names = players
        .iter()
        .map(|player| player.name.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    const TOKEN_SIGNATURES: &[(&str, &[&str])] = &[
        ("oblivion-express", &["train", "列车"]),
        ("malwarfare", &["aurana", "奥罗娜"]),
        ("the-vermillion-problem", &["vermillion", "维米利恩"]),
        (
            "chain-of-ascension",
            &["jinara", "吉娜拉", "malash", "马拉什"],
        ),
        ("mist-opportunities", &["terrazine", "地嗪"]),
        ("void-launch", &["shuttle", "穿梭机"]),
        ("miner-evacuation", &["evacuation", "撤离", "矿工"]),
        ("dead-of-night", &["infested", "感染"]),
        ("part-and-parcel", &["balius", "巴利俄斯", "部件"]),
        ("cradle-of-death", &["artifact truck", "神器卡车"]),
        ("lock-and-load", &["celestial lock", "天锁"]),
        ("temple-of-the-past", &["temple", "神庙"]),
        ("void-rifts", &["void rift", "虚空撕裂者"]),
        ("rifts-to-korhal", &["korhal", "克哈"]),
        ("scythe-of-amon", &["scythe", "黑暗杀星"]),
    ];
    if let Some((map_id, _)) = TOKEN_SIGNATURES
        .iter()
        .find(|(_, tokens)| tokens.iter().any(|token| names.contains(token)))
    {
        return Some(*map_id);
    }

    match players.len() {
        4 => Some("rifts-to-korhal"),
        11 => Some("lock-and-load"),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct RawGame {
    #[serde(rename = "displayTime")]
    display_time: Option<f64>,
    #[serde(default)]
    players: Vec<RawPlayer>,
}

#[derive(Debug, Deserialize)]
struct RawPlayer {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawUi {
    #[serde(rename = "activeScreens", default)]
    active_screens: Vec<String>,
}

#[derive(Debug, Error)]
pub enum Sc2NormalizeError {
    #[error("invalid SC2 6119 JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SC2 displayTime is missing, negative, or non-finite")]
    InvalidDisplayTime,
}

#[derive(Debug, Error)]
pub enum Sc2EndpointError {
    #[error("could not build SC2 6119 HTTP client: {0}")]
    ClientBuild(String),
    #[error("SC2 6119 transport error: {0}")]
    Transport(String),
    #[error("SC2 6119 returned HTTP status {0}")]
    HttpStatus(u16),
}
