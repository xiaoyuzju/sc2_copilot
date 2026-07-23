use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use sc2_copilot_core::{
    AlertBatch, CopilotEngine, EngineInput, EngineSettings, EngineUpdate, EngineView, EventTiming,
    Fact, GameObservation, LocationSpec, ScheduleCatalog, Trigger, UserCommand,
};
use sc2_copilot_vision::{VisionEvidence, VisionUpdate};

use crate::{AlertPlayer, AppSettings, Sc2Observation, Sc2Poll};

const DIAGNOSTIC_LIMIT: usize = 20;
const TEMPLE_OF_THE_PAST_MAP_ID: &str = "temple-of-the-past";
const TEMPLE_OF_THE_PAST_DEFAULT_VARIANT_ID: &str = "layout-b";
type MutatorEventDetails = HashMap<String, BTreeMap<String, Vec<String>>>;

struct CatalogDescription {
    maps: Vec<MapDescriptor>,
    event_labels: HashMap<String, String>,
    mutator_event_details: MutatorEventDetails,
}

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
pub struct MutatorDescriptor {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapDescriptor {
    pub id: String,
    pub display_name: String,
    pub variants: Vec<VariantDescriptor>,
    pub mutators: Vec<MutatorDescriptor>,
    pub stage_ids: Vec<String>,
    pub unsupported_row_count: usize,
    pub unsupported_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertCard {
    pub timing: EventTiming,
    pub event_ids: Vec<String>,
    pub event_labels: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControllerUpdate {
    pub new_alerts: Vec<AlertCard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariantSelectionSource {
    Default,
    Vision,
    Manual,
}

pub struct AppController {
    engine: CopilotEngine,
    maps: Vec<MapDescriptor>,
    event_labels: HashMap<String, String>,
    mutator_event_details: MutatorEventDetails,
    player: Box<dyn AlertPlayer>,
    connection: ConnectionState,
    engine_view: EngineView,
    auto_map_id: Option<String>,
    manual_map_id: Option<String>,
    current_session_id: Option<String>,
    current_game_time_milliseconds: Option<u64>,
    variant_selection_source: Option<VariantSelectionSource>,
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
        let description = describe_catalog(&catalog);
        let snapshot_batch = catalog.snapshot_batch().to_owned();
        let engine_settings = EngineSettings {
            lead_time_milliseconds: settings.lead_time_seconds.saturating_mul(1_000),
        };
        Self {
            engine: CopilotEngine::new(catalog, engine_settings),
            maps: description.maps,
            event_labels: description.event_labels,
            mutator_event_details: description.mutator_event_details,
            player,
            connection: ConnectionState::Disconnected,
            engine_view: EngineView::default(),
            auto_map_id: None,
            manual_map_id: None,
            current_session_id: None,
            current_game_time_milliseconds: None,
            variant_selection_source: None,
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
                self.variant_selection_source = None;
                self.apply_engine(EngineInput::Observation(GameObservation::Menu))
            }
            Sc2Observation::InGame {
                session_id,
                map_id,
                game_time_milliseconds,
                ..
            } => {
                let previous_map_id = self.selected_map_id().map(str::to_owned);
                let is_new_session = self.current_session_id.as_deref() != Some(&session_id);
                self.connection = ConnectionState::InGame;
                self.auto_map_id = map_id;
                self.current_session_id = Some(session_id.clone());
                self.current_game_time_milliseconds = Some(game_time_milliseconds);
                if is_new_session || previous_map_id.as_deref() != self.selected_map_id() {
                    self.variant_selection_source = None;
                }
                let Some(map_id) = self.selected_map_id().map(str::to_owned) else {
                    self.record_diagnostic(
                        "无法从 6119 玩家列表唯一识别地图，请在设置中手动选择当前地图",
                    );
                    return ControllerUpdate::default();
                };
                self.apply_in_game_observation(session_id, map_id, game_time_milliseconds)
            }
        }
    }

    pub fn select_manual_map(&mut self, map_id: Option<String>) -> ControllerUpdate {
        let previous_map_id = self.selected_map_id().map(str::to_owned);
        self.manual_map_id = map_id;
        if previous_map_id.as_deref() != self.selected_map_id() {
            self.variant_selection_source = None;
        }
        self.refresh_current_observation()
    }

    pub fn select_variant(&mut self, variant_id: Option<String>) -> ControllerUpdate {
        self.variant_selection_source = variant_id.as_ref().map(|_| VariantSelectionSource::Manual);
        self.apply_engine(EngineInput::Command(UserCommand::SelectVariant {
            variant_id,
        }))
    }

    pub fn handle_vision(&mut self, update: VisionUpdate) -> ControllerUpdate {
        if self.connection != ConnectionState::InGame
            || self.variant_selection_source == Some(VariantSelectionSource::Manual)
            || self.current_session_id.as_deref() != Some(&update.session_id)
            || self.selected_map_id() != Some(&update.map_id)
        {
            return ControllerUpdate::default();
        }

        match update.evidence {
            VisionEvidence::MapVariant { variant_id } => {
                let variant_exists = self
                    .map(&update.map_id)
                    .is_some_and(|map| map.variants.iter().any(|variant| variant.id == variant_id));
                if !variant_exists {
                    return ControllerUpdate::default();
                }
                let result = self.apply_engine(EngineInput::Command(UserCommand::SelectVariant {
                    variant_id: Some(variant_id),
                }));
                self.variant_selection_source = Some(VariantSelectionSource::Vision);
                result
            }
        }
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

    pub fn set_mutator_active(&mut self, mutator_id: String, active: bool) -> ControllerUpdate {
        self.apply_engine(EngineInput::Command(UserCommand::SetMutatorActive {
            mutator_id,
            active,
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

    pub(crate) fn observed_session_id(&self) -> Option<&str> {
        self.current_session_id.as_deref()
    }

    pub(crate) fn observed_game_time_milliseconds(&self) -> Option<u64> {
        self.current_game_time_milliseconds
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
        let mut label = self
            .event_labels
            .get(event_id)
            .map(String::as_str)
            .unwrap_or(event_id)
            .to_owned();
        if let Some(details_by_mutator) = self.mutator_event_details.get(event_id) {
            for mutator_id in &self.engine_view.active_mutator_ids {
                if let Some(details) = details_by_mutator.get(mutator_id) {
                    for detail in details {
                        label.push_str(" · ");
                        label.push_str(detail);
                    }
                }
            }
        }
        label
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
        self.apply_in_game_observation(session_id, map_id, game_time_milliseconds)
    }

    fn apply_in_game_observation(
        &mut self,
        session_id: String,
        map_id: String,
        game_time_milliseconds: u64,
    ) -> ControllerUpdate {
        let should_default_variant = map_id == TEMPLE_OF_THE_PAST_MAP_ID
            && (self.engine_view.session_id.as_deref() != Some(&session_id)
                || self.engine_view.map_id.as_deref() != Some(&map_id));
        let mut update = self.apply_engine(EngineInput::Observation(GameObservation::InGame {
            session_id,
            map_id,
            game_time_milliseconds,
        }));
        if should_default_variant {
            let default_update =
                self.apply_engine(EngineInput::Command(UserCommand::SelectVariant {
                    variant_id: Some(TEMPLE_OF_THE_PAST_DEFAULT_VARIANT_ID.to_owned()),
                }));
            self.variant_selection_source = Some(VariantSelectionSource::Default);
            update.new_alerts.extend(default_update.new_alerts);
        }
        update
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
            timing: batch.timing,
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

fn describe_catalog(catalog: &ScheduleCatalog) -> CatalogDescription {
    let mut maps = Vec::with_capacity(catalog.map_count());
    let mut labels = HashMap::new();
    let mut mutator_event_details = HashMap::new();
    for schedule in catalog.schedules() {
        let mut stage_ids = BTreeSet::new();
        let mut mutators = BTreeMap::<String, String>::new();
        let mut unsupported_reasons = BTreeMap::<String, usize>::new();
        for event in schedule.events() {
            labels.insert(event.id().to_owned(), describe_event(event.facts()));
            let mut event_mutator_details = BTreeMap::<String, Vec<String>>::new();
            for fact in event.facts() {
                if let Fact::MutatorContext {
                    mutator_id,
                    display_name,
                    label,
                    value,
                } = fact
                {
                    mutators.insert(mutator_id.clone(), display_name.clone());
                    event_mutator_details
                        .entry(mutator_id.clone())
                        .or_default()
                        .push(format!("{label} {value}"));
                }
            }
            if !event_mutator_details.is_empty() {
                mutator_event_details.insert(event.id().to_owned(), event_mutator_details);
            }
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
            mutators: mutators
                .into_iter()
                .map(|(id, display_name)| MutatorDescriptor { id, display_name })
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
    CatalogDescription {
        maps,
        event_labels: labels,
        mutator_event_details,
    }
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
        Fact::WaveExpression { value } => Some(format!("波次 {value}")),
        Fact::Location { value } => Some(describe_location(value)),
        Fact::Target { value } => Some(value.clone()),
        Fact::Route { value } => Some(format!("路线 {value}")),
        Fact::Health { value } => Some(format!("生命值 {value}")),
        Fact::Shield { value } => Some(format!("护盾值 {value}")),
        Fact::UnitCount { unit, value } => Some(format!("{unit:?} × {value}")),
        Fact::Count { subject, value } => Some(format!("{subject} {value}")),
        Fact::ScaleLevel { value } => Some(format!("规模 {value}")),
        Fact::ScaleExpression { value } => Some(format!("规模 {value}")),
        Fact::TechLevel { value } => Some(format!("科技 {value}")),
        Fact::TechExpression { value } => Some(format!("科技 {value}")),
        Fact::Composition { value } => Some(format!("组成 {value}")),
        Fact::Probability { value } => Some(format!("概率 {value}")),
        Fact::Detail { label, value } => Some(format!("{label} {value}")),
        Fact::MutatorContext { .. } => None,
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
