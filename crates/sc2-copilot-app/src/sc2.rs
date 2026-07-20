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
        let ui_is_in_game = ui.active_screens.is_empty();
        if game.players.is_empty() || !ui_is_in_game {
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
    MAP_SIGNATURES
        .iter()
        .find(|signature| signature.matches(players))
        .map(|signature| signature.map_id)
}

struct MapSignature {
    map_id: &'static str,
    player_count: usize,
    checks: &'static [(usize, &'static [&'static str])],
}

impl MapSignature {
    fn matches(&self, players: &[RawPlayer]) -> bool {
        players.len() == self.player_count
            && self.checks.iter().all(|(index, expected_names)| {
                players.get(*index).is_some_and(|player| {
                    let actual = normalized_player_name(&player.name);
                    expected_names
                        .iter()
                        .any(|expected| actual == normalized_player_name(expected))
                })
            })
    }
}

fn normalized_player_name(name: &str) -> String {
    name.trim().to_lowercase().replace('’', "'")
}

const MAP_SIGNATURES: &[MapSignature] = &[
    MapSignature {
        map_id: "chain-of-ascension",
        player_count: 9,
        checks: &[
            (6, &["Ji'nara", "吉娜拉", "吉娜拉"]),
            (8, &["Slayn Elemental", "斯雷恩元素生物", "史雷因元素兽"]),
        ],
    },
    MapSignature {
        map_id: "cradle-of-death",
        player_count: 6,
        checks: &[
            (4, &["Special Amon's Forces", "埃蒙的特战队"]),
            (5, &["Special Amon's Forces", "埃蒙的特战队"]),
        ],
    },
    MapSignature {
        map_id: "dead-of-night",
        player_count: 7,
        checks: &[
            (4, &["Infested", "感染体", "受到感染"]),
            (5, &["Sensor Tower", "感应塔", "感應塔"]),
        ],
    },
    MapSignature {
        map_id: "lock-and-load",
        player_count: 4,
        checks: &[
            (2, &["Amon's Forces", "埃蒙的部队", "亞蒙的軍隊"]),
            (3, &["Amon's Forces", "埃蒙的部队", "亞蒙的軍隊"]),
        ],
    },
    MapSignature {
        map_id: "malwarfare",
        player_count: 6,
        checks: &[
            (
                2,
                &["Purifier Hologram", "净化者全息投影", "淨化者全像部隊"],
            ),
            (4, &["Megalith", "麦加利斯", "碩像儀"]),
        ],
    },
    MapSignature {
        map_id: "miner-evacuation",
        player_count: 11,
        checks: &[
            (7, &["Kel-Morian Miners", "凯莫瑞安矿工", "凱爾莫瑞亞礦工"]),
            (9, &["Kel-Morian Miners", "凯莫瑞安矿工", "凱爾莫瑞亞礦工"]),
        ],
    },
    MapSignature {
        map_id: "mist-opportunities",
        player_count: 6,
        checks: &[
            (4, &["Terrazine", "地嗪", "態化氫"]),
            (5, &["Egon Stetmann", "艾贡·斯台特曼", "伊崗‧斯特曼"]),
        ],
    },
    MapSignature {
        map_id: "oblivion-express",
        player_count: 7,
        checks: &[
            (5, &["Amon's Forces", "埃蒙的部队", "亞蒙的軍隊"]),
            (6, &["Amon's Forces", "埃蒙的部队", "亞蒙的軍隊"]),
        ],
    },
    MapSignature {
        map_id: "part-and-parcel",
        player_count: 6,
        checks: &[
            (4, &["Balius", "巴利俄斯", "巴流斯"]),
            (5, &["Moebius Train", "莫比斯列车", "莫比斯列車"]),
        ],
    },
    MapSignature {
        map_id: "rifts-to-korhal",
        player_count: 6,
        checks: &[
            (4, &["Void Shard", "虚空碎片", "虛空晶體"]),
            (5, &["Pirates", "海盗", "海盜"]),
        ],
    },
    MapSignature {
        map_id: "scythe-of-amon",
        player_count: 8,
        checks: &[
            (6, &["Amon's Forces", "埃蒙的部队", "亞蒙的軍隊"]),
            (7, &["Evacuees", "待救者", "待撤離人員"]),
        ],
    },
    MapSignature {
        map_id: "temple-of-the-past",
        player_count: 9,
        checks: &[(6, &["Temple", "神庙", "神殿"]), (8, &["Rocks", "岩石"])],
    },
    MapSignature {
        map_id: "the-vermillion-problem",
        player_count: 8,
        checks: &[
            (6, &["Molten Salamander", "熔岩蜥蜴", "熔岩巨蜥"]),
            (7, &["Civilians", "平民"]),
        ],
    },
    MapSignature {
        map_id: "void-launch",
        player_count: 7,
        checks: &[
            (5, &["Warp Conduit", "时空航道", "躍傳中繼站"]),
            (6, &["Scientific Research Team", "科研队伍", "科學研發隊"]),
        ],
    },
    MapSignature {
        map_id: "void-rifts",
        player_count: 6,
        checks: &[
            (4, &["Amon's Forces", "埃蒙的部队", "亞蒙的軍隊"]),
            (
                5,
                &["Sgt. Hammer's Forces", "重锤军士的部队", "榔頭中士的部隊"],
            ),
        ],
    },
];

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
