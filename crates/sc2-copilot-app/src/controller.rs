use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use sc2_copilot_core::{
    AlertBatch, CopilotEngine, EngineInput, EngineSettings, EngineUpdate, EngineView, Fact,
    GameObservation, LocationSpec, ScheduleCatalog, Trigger, UserCommand,
};

use crate::{AlertPlayer, AppSettings, Sc2Observation, Sc2Poll};

const DIAGNOSTIC_LIMIT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Menu,
    InGame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantDescriptor {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapDescriptor {
    pub id: String,
    pub display_name: String,
    pub variants: Vec<VariantDescriptor>,
    pub stage_ids: Vec<String>,
    pub unsupported_row_count: usize,
    pub unsupported_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertCard {
    pub event_time_milliseconds: u64,
    pub event_ids: Vec<String>,
    pub event_labels: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControllerUpdate {
    pub new_alerts: Vec<AlertCard>,
}

pub struct AppController {
    engine: CopilotEngine,
    maps: Vec<MapDescriptor>,
    event_labels: HashMap<String, String>,
    player: Box<dyn AlertPlayer>,
    connection: ConnectionState,
    engine_view: EngineView,
    auto_map_id: Option<String>,
    manual_map_id: Option<String>,
    current_session_id: Option<String>,
    current_game_time_milliseconds: Option<u64>,
    diagnostics: VecDeque<String>,
    settings: AppSettings,
    snapshot_batch: String,
}

impl AppController {
    pub fn new(
        catalog: ScheduleCatalog,
        settings: AppSettings,
        player: Box<dyn AlertPlayer>,
    ) -> Self {
        let (maps, event_labels) = describe_catalog(&catalog);
        let snapshot_batch = catalog.snapshot_batch().to_owned();
        let engine_settings = EngineSettings {
            lead_time_milliseconds: settings.lead_time_seconds.saturating_mul(1_000),
        };
        Self {
            engine: CopilotEngine::new(catalog, engine_settings),
            maps,
            event_labels,
            player,
            connection: ConnectionState::Disconnected,
            engine_view: EngineView::default(),
            auto_map_id: None,
            manual_map_id: None,
            current_session_id: None,
            current_game_time_milliseconds: None,
            diagnostics: VecDeque::new(),
            settings,
            snapshot_batch,
        }
    }

    pub fn handle_poll(&mut self, poll: Sc2Poll) -> ControllerUpdate {
        if let Some(diagnostic) = poll.diagnostic {
            self.record_diagnostic(diagnostic);
        }
        match poll.observation {
            Sc2Observation::Disconnected => {
                self.connection = ConnectionState::Disconnected;
                self.engine
                    .apply(EngineInput::Observation(GameObservation::Disconnected));
                ControllerUpdate::default()
            }
            Sc2Observation::Menu => {
                self.connection = ConnectionState::Menu;
                self.auto_map_id = None;
                self.manual_map_id = None;
                self.current_session_id = None;
                self.current_game_time_milliseconds = None;
                self.apply_engine(EngineInput::Observation(GameObservation::Menu))
            }
            Sc2Observation::InGame {
                session_id,
                map_id,
                game_time_milliseconds,
                ..
            } => {
                self.connection = ConnectionState::InGame;
                self.auto_map_id = map_id;
                self.current_session_id = Some(session_id.clone());
                self.current_game_time_milliseconds = Some(game_time_milliseconds);
                let Some(map_id) = self.selected_map_id().map(str::to_owned) else {
                    self.record_diagnostic(
                        "无法从 6119 玩家列表唯一识别地图，请在设置中手动选择当前地图",
                    );
                    return ControllerUpdate::default();
                };
                self.apply_engine(EngineInput::Observation(GameObservation::InGame {
                    session_id,
                    map_id,
                    game_time_milliseconds,
                }))
            }
        }
    }

    pub fn select_manual_map(&mut self, map_id: Option<String>) -> ControllerUpdate {
        self.manual_map_id = map_id;
        self.refresh_current_observation()
    }

    pub fn select_variant(&mut self, variant_id: Option<String>) -> ControllerUpdate {
        self.apply_engine(EngineInput::Command(UserCommand::SelectVariant {
            variant_id,
        }))
    }

    pub fn set_stage_anchor(&mut self, stage_id: String) -> ControllerUpdate {
        self.apply_engine(EngineInput::Command(UserCommand::SetStageAnchor {
            stage_id,
        }))
    }

    pub fn clear_stage_anchor(&mut self, stage_id: String) -> ControllerUpdate {
        self.apply_engine(EngineInput::Command(UserCommand::ClearStageAnchor {
            stage_id,
        }))
    }

    pub fn update_settings(&mut self, settings: AppSettings) -> ControllerUpdate {
        self.settings = settings;
        self.apply_engine(EngineInput::SettingsChanged(EngineSettings {
            lead_time_milliseconds: self.settings.lead_time_seconds.saturating_mul(1_000),
        }))
    }

    pub fn record_external_diagnostic(&mut self, diagnostic: impl Into<String>) {
        self.record_diagnostic(diagnostic.into());
    }

    pub fn maps(&self) -> &[MapDescriptor] {
        &self.maps
    }

    pub fn map(&self, map_id: &str) -> Option<&MapDescriptor> {
        self.maps.iter().find(|map| map.id == map_id)
    }

    pub fn selected_map_id(&self) -> Option<&str> {
        self.auto_map_id
            .as_deref()
            .or(self.manual_map_id.as_deref())
    }

    pub fn auto_map_id(&self) -> Option<&str> {
        self.auto_map_id.as_deref()
    }

    pub fn manual_map_id(&self) -> Option<&str> {
        self.manual_map_id.as_deref()
    }

    pub fn connection(&self) -> ConnectionState {
        self.connection
    }

    pub fn view(&self) -> &EngineView {
        &self.engine_view
    }

    pub fn diagnostics(&self) -> &VecDeque<String> {
        &self.diagnostics
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn snapshot_batch(&self) -> &str {
        &self.snapshot_batch
    }

    pub fn player_status(&self) -> &str {
        self.player.status()
    }

    pub fn event_label(&self, event_id: &str) -> String {
        self.event_labels
            .get(event_id)
            .map(String::as_str)
            .unwrap_or(event_id)
            .to_owned()
    }

    fn refresh_current_observation(&mut self) -> ControllerUpdate {
        let Some(session_id) = self.current_session_id.clone() else {
            return ControllerUpdate::default();
        };
        let Some(map_id) = self.selected_map_id().map(str::to_owned) else {
            return ControllerUpdate::default();
        };
        let Some(game_time_milliseconds) = self.current_game_time_milliseconds else {
            return ControllerUpdate::default();
        };
        self.apply_engine(EngineInput::Observation(GameObservation::InGame {
            session_id,
            map_id,
            game_time_milliseconds,
        }))
    }

    fn apply_engine(&mut self, input: EngineInput) -> ControllerUpdate {
        let update = self.engine.apply(input);
        self.engine_view = update.view.clone();
        self.deliver(update)
    }

    fn deliver(&mut self, update: EngineUpdate) -> ControllerUpdate {
        let mut new_alerts = Vec::with_capacity(update.alert_batches.len());
        for batch in update.alert_batches {
            if let Err(error) = self.player.play(&batch) {
                self.record_diagnostic(format!("提醒播放失败：{error}"));
            }
            new_alerts.push(self.alert_card(&batch));
        }
        ControllerUpdate { new_alerts }
    }

    fn alert_card(&self, batch: &AlertBatch) -> AlertCard {
        AlertCard {
            event_time_milliseconds: batch.event_time_milliseconds,
            event_ids: batch.event_ids.clone(),
            event_labels: batch
                .event_ids
                .iter()
                .map(|event_id| self.event_label(event_id))
                .collect(),
        }
    }

    fn record_diagnostic(&mut self, diagnostic: impl Into<String>) {
        let diagnostic = diagnostic.into();
        if self.diagnostics.back() == Some(&diagnostic) {
            return;
        }
        self.diagnostics.push_back(diagnostic);
        while self.diagnostics.len() > DIAGNOSTIC_LIMIT {
            self.diagnostics.pop_front();
        }
    }
}

fn describe_catalog(catalog: &ScheduleCatalog) -> (Vec<MapDescriptor>, HashMap<String, String>) {
    let mut maps = Vec::with_capacity(catalog.map_count());
    let mut labels = HashMap::new();
    for schedule in catalog.schedules() {
        let mut stage_ids = BTreeSet::new();
        let mut unsupported_reasons = BTreeMap::<String, usize>::new();
        for event in schedule.events() {
            labels.insert(event.id().to_owned(), describe_event(event.facts()));
            match event.trigger() {
                Trigger::AtStageElapsed { stage_id, .. }
                | Trigger::AtStageRemaining { stage_id, .. } => {
                    stage_ids.insert(stage_id.to_owned());
                }
                Trigger::AtGameTime { .. } | Trigger::AtGameTimeWindow { .. } => {}
            }
        }
        for row in schedule
            .coverage()
            .iter()
            .flat_map(|table| table.unsupported_rows())
        {
            *unsupported_reasons
                .entry(format!("{:?}", row.reason()))
                .or_default() += 1;
        }
        maps.push(MapDescriptor {
            id: schedule.id().to_owned(),
            display_name: schedule.display_name().to_owned(),
            variants: schedule
                .variants()
                .iter()
                .map(|variant| VariantDescriptor {
                    id: variant.id().to_owned(),
                    display_name: variant.display_name().to_owned(),
                })
                .collect(),
            stage_ids: stage_ids.into_iter().collect(),
            unsupported_row_count: schedule
                .coverage()
                .iter()
                .map(|table| table.unsupported_rows().len())
                .sum(),
            unsupported_reasons: unsupported_reasons
                .into_iter()
                .map(|(reason, count)| format!("{reason}: {count}"))
                .collect(),
        });
    }
    (maps, labels)
}

fn describe_event(facts: &[Fact]) -> String {
    let descriptions = facts.iter().filter_map(describe_fact).collect::<Vec<_>>();
    if descriptions.is_empty() {
        "地图事件".to_owned()
    } else {
        descriptions.join(" · ")
    }
}

fn describe_fact(fact: &Fact) -> Option<String> {
    match fact {
        Fact::EventCategory { value } => Some(format!("{value:?}")),
        Fact::Wave { number, branch } => Some(match branch {
            Some(branch) => format!("第 {number}-{branch} 波"),
            None => format!("第 {number} 波"),
        }),
        Fact::Location { value } => Some(describe_location(value)),
        Fact::Target { value } => Some(value.clone()),
        Fact::Health { value } => Some(format!("生命值 {value}")),
        Fact::UnitCount { unit, value } => Some(format!("{unit:?} × {value}")),
        Fact::ScaleLevel { value } => Some(format!("规模 {value}")),
        Fact::TechLevel { value } => Some(format!("科技 {value}")),
    }
}

fn describe_location(location: &LocationSpec) -> String {
    match location {
        LocationSpec::Single { name } => name.clone(),
        LocationSpec::All { names } => names.join("、"),
        LocationSpec::Any { options } => options
            .iter()
            .map(|option| option.name.as_str())
            .collect::<Vec<_>>()
            .join(" / "),
    }
}
