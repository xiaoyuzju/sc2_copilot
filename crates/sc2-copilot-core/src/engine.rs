use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashSet},
};

use crate::{CompiledEvent, RuntimeSupport, ScheduleCatalog, Trigger};

const DEFAULT_LEAD_TIME_MILLISECONDS: u64 = 30_000;

#[derive(Debug)]
pub struct CopilotEngine {
    catalog: ScheduleCatalog,
    settings: EngineSettings,
    session: Option<SessionState>,
}

impl CopilotEngine {
    pub fn new(catalog: ScheduleCatalog, settings: EngineSettings) -> Self {
        Self {
            catalog,
            settings,
            session: None,
        }
    }

    pub fn apply(&mut self, input: EngineInput) -> EngineUpdate {
        match input {
            EngineInput::Observation(GameObservation::InGame {
                session_id,
                map_id,
                game_time_milliseconds,
            }) => self.observe_in_game(session_id, map_id, game_time_milliseconds),
            EngineInput::Observation(GameObservation::Menu) => {
                self.session = None;
                EngineUpdate::idle()
            }
            EngineInput::Observation(GameObservation::Disconnected) => EngineUpdate::idle(),
            EngineInput::SettingsChanged(settings) => {
                self.settings = settings;
                self.refresh_current_session()
            }
            EngineInput::Command(UserCommand::SelectVariant { variant_id }) => {
                if let Some(session) = &mut self.session {
                    session.variant_id = variant_id;
                }
                self.refresh_current_session()
            }
            EngineInput::Command(UserCommand::SetStageAnchor { stage_id }) => {
                if let Some(session) = &mut self.session {
                    session
                        .stage_anchors
                        .insert(stage_id, session.game_time_milliseconds);
                }
                self.refresh_current_session()
            }
            EngineInput::Command(UserCommand::ClearStageAnchor { stage_id }) => {
                if let Some(session) = &mut self.session {
                    session.stage_anchors.remove(&stage_id);
                }
                self.refresh_current_session()
            }
            EngineInput::Command(UserCommand::SetMutatorActive { mutator_id, active }) => {
                if let Some(session) = &mut self.session {
                    if active {
                        session.active_mutator_ids.insert(mutator_id);
                    } else {
                        session.active_mutator_ids.remove(&mutator_id);
                    }
                }
                self.refresh_current_session()
            }
        }
    }

    fn refresh_current_session(&mut self) -> EngineUpdate {
        let observation = self.session.as_ref().map(|session| {
            (
                session.session_id.clone(),
                session.map_id.clone(),
                session.game_time_milliseconds,
            )
        });
        match observation {
            Some((session_id, map_id, game_time_milliseconds)) => {
                self.observe_in_game(session_id, map_id, game_time_milliseconds)
            }
            None => EngineUpdate::idle(),
        }
    }

    fn observe_in_game(
        &mut self,
        session_id: String,
        map_id: String,
        game_time_milliseconds: u64,
    ) -> EngineUpdate {
        let starts_new_session = self.session.as_ref().is_none_or(|session| {
            session.session_id != session_id
                || session.map_id != map_id
                || (game_time_milliseconds == 0 && session.game_time_milliseconds > 0)
        });
        if starts_new_session {
            self.session = Some(SessionState::new(session_id, map_id));
        }

        let session = self.session.as_mut().expect("session was initialized");
        session.game_time_milliseconds = game_time_milliseconds;

        let mut batches = BTreeMap::<EventTiming, Vec<String>>::new();
        let mut newly_missed_event_ids = Vec::new();
        let mut upcoming_events = Vec::new();
        if let Some(schedule) = self.catalog.schedule_for(&session.map_id, None) {
            for event in schedule.events() {
                let resolved_timing = resolve_event_timing(event.trigger(), session);
                if !event_is_active(event, session)
                    || resolved_timing.is_none()
                    || session.notified.contains(event.id())
                    || session.missed.contains(event.id())
                {
                    continue;
                }
                let timing = resolved_timing.expect("resolved timing was checked");

                if timing.has_ended(game_time_milliseconds) {
                    session.missed.insert(event.id().to_owned());
                    newly_missed_event_ids.push(event.id().to_owned());
                    continue;
                }
                if game_time_milliseconds
                    >= timing
                        .earliest_milliseconds()
                        .saturating_sub(self.settings.lead_time_milliseconds)
                {
                    session.notified.insert(event.id().to_owned());
                    batches
                        .entry(timing)
                        .or_default()
                        .push(event.id().to_owned());
                }
            }

            for event in schedule.events() {
                if !event_is_active(event, session) || session.missed.contains(event.id()) {
                    continue;
                }
                let Some(timing) = resolve_event_timing(event.trigger(), session) else {
                    continue;
                };
                if !timing.has_ended(game_time_milliseconds) {
                    upcoming_events.push(ScheduledEventView {
                        event_id: event.id().to_owned(),
                        timing,
                        remaining_milliseconds: timing
                            .earliest_milliseconds()
                            .saturating_sub(game_time_milliseconds),
                    });
                }
            }
        }
        upcoming_events.sort_by(|left, right| {
            left.timing
                .cmp(&right.timing)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });

        let alert_batches = batches
            .into_iter()
            .map(|(timing, mut event_ids)| {
                event_ids.sort();
                AlertBatch { timing, event_ids }
            })
            .collect();

        EngineUpdate {
            alert_batches,
            newly_missed_event_ids,
            view: EngineView {
                session_id: Some(session.session_id.clone()),
                map_id: Some(session.map_id.clone()),
                game_time_milliseconds: Some(game_time_milliseconds),
                variant_id: session.variant_id.clone(),
                active_mutator_ids: session.active_mutator_ids.iter().cloned().collect(),
                stage_anchors: session
                    .stage_anchors
                    .iter()
                    .map(|(stage_id, game_time_milliseconds)| StageAnchorView {
                        stage_id: stage_id.clone(),
                        game_time_milliseconds: *game_time_milliseconds,
                    })
                    .collect(),
                upcoming_events,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineSettings {
    pub lead_time_milliseconds: u64,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            lead_time_milliseconds: DEFAULT_LEAD_TIME_MILLISECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineInput {
    Observation(GameObservation),
    SettingsChanged(EngineSettings),
    Command(UserCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCommand {
    SelectVariant { variant_id: Option<String> },
    SetStageAnchor { stage_id: String },
    ClearStageAnchor { stage_id: String },
    SetMutatorActive { mutator_id: String, active: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameObservation {
    Disconnected,
    Menu,
    InGame {
        session_id: String,
        map_id: String,
        game_time_milliseconds: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertBatch {
    pub timing: EventTiming,
    pub event_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTiming {
    Exact {
        milliseconds: u64,
    },
    Window {
        earliest_milliseconds: u64,
        latest_milliseconds: u64,
    },
}

impl EventTiming {
    pub fn earliest_milliseconds(self) -> u64 {
        match self {
            Self::Exact { milliseconds } => milliseconds,
            Self::Window {
                earliest_milliseconds,
                ..
            } => earliest_milliseconds,
        }
    }

    pub fn latest_milliseconds(self) -> u64 {
        match self {
            Self::Exact { milliseconds } => milliseconds,
            Self::Window {
                latest_milliseconds,
                ..
            } => latest_milliseconds,
        }
    }

    fn has_ended(self, game_time_milliseconds: u64) -> bool {
        self.latest_milliseconds() <= game_time_milliseconds
    }

    fn sort_key(self) -> (u64, u64, u8) {
        match self {
            Self::Exact { milliseconds } => (milliseconds, milliseconds, 0),
            Self::Window {
                earliest_milliseconds,
                latest_milliseconds,
            } => (earliest_milliseconds, latest_milliseconds, 1),
        }
    }
}

impl Ord for EventTiming {
    fn cmp(&self, other: &Self) -> Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for EventTiming {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineUpdate {
    pub alert_batches: Vec<AlertBatch>,
    pub newly_missed_event_ids: Vec<String>,
    pub view: EngineView,
}

impl EngineUpdate {
    fn idle() -> Self {
        Self {
            alert_batches: Vec::new(),
            newly_missed_event_ids: Vec::new(),
            view: EngineView::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineView {
    pub session_id: Option<String>,
    pub map_id: Option<String>,
    pub game_time_milliseconds: Option<u64>,
    pub variant_id: Option<String>,
    pub active_mutator_ids: Vec<String>,
    pub stage_anchors: Vec<StageAnchorView>,
    pub upcoming_events: Vec<ScheduledEventView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageAnchorView {
    pub stage_id: String,
    pub game_time_milliseconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEventView {
    pub event_id: String,
    pub timing: EventTiming,
    pub remaining_milliseconds: u64,
}

#[derive(Debug)]
struct SessionState {
    session_id: String,
    map_id: String,
    game_time_milliseconds: u64,
    variant_id: Option<String>,
    active_mutator_ids: BTreeSet<String>,
    stage_anchors: BTreeMap<String, u64>,
    notified: HashSet<String>,
    missed: HashSet<String>,
}

impl SessionState {
    fn new(session_id: String, map_id: String) -> Self {
        Self {
            session_id,
            map_id,
            game_time_milliseconds: 0,
            variant_id: None,
            active_mutator_ids: BTreeSet::new(),
            stage_anchors: BTreeMap::new(),
            notified: HashSet::new(),
            missed: HashSet::new(),
        }
    }
}

fn resolve_event_timing(trigger: &Trigger, session: &SessionState) -> Option<EventTiming> {
    match trigger {
        Trigger::AtGameTime { milliseconds } => Some(EventTiming::Exact {
            milliseconds: *milliseconds,
        }),
        Trigger::AtGameTimeWindow {
            earliest_milliseconds,
            latest_milliseconds,
        } => Some(EventTiming::Window {
            earliest_milliseconds: *earliest_milliseconds,
            latest_milliseconds: *latest_milliseconds,
        }),
        Trigger::AtStageElapsed {
            stage_id,
            milliseconds,
        } => session
            .stage_anchors
            .get(stage_id)
            .map(|anchor| EventTiming::Exact {
                milliseconds: anchor.saturating_add(*milliseconds),
            }),
        Trigger::AtStageRemaining { .. } => None,
    }
}

fn event_is_active(event: &CompiledEvent, session: &SessionState) -> bool {
    let variant_is_active = event
        .variant_id()
        .is_none_or(|variant| session.variant_id.as_deref() == Some(variant));
    let runtime_is_active = match event.runtime_support() {
        RuntimeSupport::Automatic => true,
        RuntimeSupport::ManualContext => {
            event.variant_id().is_some()
                || matches!(
                    event.trigger(),
                    Trigger::AtStageElapsed { .. } | Trigger::AtStageRemaining { .. }
                )
        }
        RuntimeSupport::Unsupported => false,
    };
    variant_is_active && runtime_is_active
}
