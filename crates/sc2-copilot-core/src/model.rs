use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionSchedule {
    map_id: String,
    display_name: String,
    events: Vec<CompiledEvent>,
}

impl MissionSchedule {
    pub fn id(&self) -> &str {
        &self.map_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn events(&self) -> &[CompiledEvent] {
        &self.events
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledEvent {
    map_id: String,
    #[serde(default)]
    variant_id: Option<String>,
    event_id: String,
    trigger: Trigger,
    facts: Vec<Fact>,
    source_refs: Vec<SourceRef>,
    runtime_support: RuntimeSupport,
}

impl CompiledEvent {
    pub fn map_id(&self) -> &str {
        &self.map_id
    }

    pub fn variant_id(&self) -> Option<&str> {
        self.variant_id.as_deref()
    }

    pub fn id(&self) -> &str {
        &self.event_id
    }

    pub fn trigger(&self) -> &Trigger {
        &self.trigger
    }

    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }

    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    pub fn runtime_support(&self) -> RuntimeSupport {
        self.runtime_support
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    AtGameTime { milliseconds: u64 },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Fact {
    EventCategory {
        value: EventCategory,
    },
    Wave {
        number: u16,
        #[serde(default)]
        branch: Option<u16>,
    },
    Location {
        value: LocationSpec,
    },
    Target {
        value: String,
    },
    Health {
        value: u32,
    },
    UnitCount {
        unit: UnitKind,
        value: u16,
    },
    ScaleLevel {
        value: u8,
    },
    TechLevel {
        value: u8,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocationSpec {
    Single { name: String },
    All { names: Vec<String> },
    Any { options: Vec<WeightedLocation> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightedLocation {
    pub name: String,
    pub weight_percent: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    TrainCar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    AttackWave,
    MainObjective,
    BonusObjective,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub source_url: String,
    pub snapshot_batch: String,
    pub snapshot_path: String,
    pub table_index: usize,
    pub row_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSupport {
    Automatic,
    ManualContext,
    Unsupported,
}
