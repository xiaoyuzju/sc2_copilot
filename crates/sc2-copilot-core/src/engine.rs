use std::collections::{BTreeMap, HashSet};

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
            EngineInput::Command(UserCommand::SelectVariant { variant_id }) => {
                let observation = self.session.as_mut().map(|session| {
                    session.variant_id = variant_id;
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
            EngineInput::Command(UserCommand::SetStageAnchor { stage_id }) => {
                let observation = self.session.as_mut().map(|session| {
                    session
                        .stage_anchors
                        .insert(stage_id, session.game_time_milliseconds);
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
            EngineInput::Command(UserCommand::ClearStageAnchor { stage_id }) => {
                let observation = self.session.as_mut().map(|session| {
                    session.stage_anchors.remove(&stage_id);
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

        let mut batches = BTreeMap::<u64, Vec<String>>::new();
        let mut newly_missed_event_ids = Vec::new();
        let mut upcoming_events = Vec::new();
        if let Some(schedule) = self.catalog.schedule_for(&session.map_id, None) {
            for event in schedule.events() {
                let resolved_time = resolve_event_time(event.trigger(), session);
                if !event_is_active(event, session)
                    || resolved_time.is_none()
                    || session.notified.contains(event.id())
                    || session.missed.contains(event.id())
                {
                    continue;
                }
                let milliseconds = resolved_time.expect("resolved time was checked");

                if milliseconds <= game_time_milliseconds {
                    session.missed.insert(event.id().to_owned());
                    newly_missed_event_ids.push(event.id().to_owned());
                    continue;
                }
                if game_time_milliseconds
                    >= milliseconds.saturating_sub(self.settings.lead_time_milliseconds)
                {
                    session.notified.insert(event.id().to_owned());
                    batches
                        .entry(milliseconds)
                        .or_default()
                        .push(event.id().to_owned());
                }
            }

            for event in schedule.events() {
                if !event_is_active(event, session) || session.missed.contains(event.id()) {
                    continue;
                }
                let Some(event_time_milliseconds) = resolve_event_time(event.trigger(), session)
                else {
                    continue;
                };
                if event_time_milliseconds > game_time_milliseconds {
                    upcoming_events.push(ScheduledEventView {
                        event_id: event.id().to_owned(),
                        event_time_milliseconds,
                        remaining_milliseconds: event_time_milliseconds
                            .saturating_sub(game_time_milliseconds),
                    });
                }
            }
        }
        upcoming_events.sort_by(|left, right| {
            left.event_time_milliseconds
                .cmp(&right.event_time_milliseconds)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });

        let alert_batches = batches
            .into_iter()
            .map(|(event_time_milliseconds, mut event_ids)| {
                event_ids.sort();
                AlertBatch {
                    event_time_milliseconds,
                    event_ids,
                }
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
    pub event_time_milliseconds: u64,
    pub event_ids: Vec<String>,
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
    pub event_time_milliseconds: u64,
    pub remaining_milliseconds: u64,
}

#[derive(Debug)]
struct SessionState {
    session_id: String,
    map_id: String,
    game_time_milliseconds: u64,
    variant_id: Option<String>,
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
            stage_anchors: BTreeMap::new(),
            notified: HashSet::new(),
            missed: HashSet::new(),
        }
    }
}

fn resolve_event_time(trigger: &Trigger, session: &SessionState) -> Option<u64> {
    match trigger {
        Trigger::AtGameTime { milliseconds } => Some(*milliseconds),
        Trigger::AtGameTimeWindow {
            earliest_milliseconds,
            ..
        } => Some(*earliest_milliseconds),
        Trigger::AtStageElapsed {
            stage_id,
            milliseconds,
        } => session
            .stage_anchors
            .get(stage_id)
            .map(|anchor| anchor.saturating_add(*milliseconds)),
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
