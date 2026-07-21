use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionSchedule {
    map_id: String,
    display_name: String,
    #[serde(default)]
    variants: Vec<ScheduleVariant>,
    events: Vec<CompiledEvent>,
    #[serde(default)]
    coverage: Vec<TableCoverage>,
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

    pub fn variants(&self) -> &[ScheduleVariant] {
        &self.variants
    }

    pub fn coverage(&self) -> &[TableCoverage] {
        &self.coverage
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleVariant {
    variant_id: String,
    display_name: String,
}

impl ScheduleVariant {
    pub fn id(&self) -> &str {
        &self.variant_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
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
    AtGameTime {
        milliseconds: u64,
    },
    AtGameTimeWindow {
        earliest_milliseconds: u64,
        latest_milliseconds: u64,
    },
    AtStageElapsed {
        stage_id: String,
        milliseconds: u64,
    },
    AtStageRemaining {
        stage_id: String,
        milliseconds: u64,
    },
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
    WaveExpression {
        value: String,
    },
    Location {
        value: LocationSpec,
    },
    Target {
        value: String,
    },
    Route {
        value: String,
    },
    Health {
        value: u32,
    },
    Shield {
        value: u32,
    },
    UnitCount {
        unit: UnitKind,
        value: u16,
    },
    Count {
        subject: String,
        value: String,
    },
    ScaleLevel {
        value: u8,
    },
    ScaleExpression {
        value: String,
    },
    TechLevel {
        value: u8,
    },
    TechExpression {
        value: String,
    },
    Composition {
        value: String,
    },
    Probability {
        value: String,
    },
    Detail {
        label: String,
        value: String,
    },
    MutatorContext {
        mutator_id: String,
        display_name: String,
        label: String,
        value: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    TrainCar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSupport {
    Automatic,
    ManualContext,
    Unsupported,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableCoverage {
    source_url: String,
    snapshot_batch: String,
    snapshot_path: String,
    table_index: usize,
    runtime_support: RuntimeSupport,
    #[serde(default)]
    unsupported_rows: Vec<UnsupportedRow>,
}

impl TableCoverage {
    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn snapshot_batch(&self) -> &str {
        &self.snapshot_batch
    }

    pub fn snapshot_path(&self) -> &str {
        &self.snapshot_path
    }

    pub fn table_index(&self) -> usize {
        self.table_index
    }

    pub fn runtime_support(&self) -> RuntimeSupport {
        self.runtime_support
    }

    pub fn unsupported_rows(&self) -> &[UnsupportedRow] {
        &self.unsupported_rows
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedRow {
    row_index: usize,
    reason: UnsupportedReason,
}

impl UnsupportedRow {
    pub fn row_index(&self) -> usize {
        self.row_index
    }

    pub fn reason(&self) -> UnsupportedReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedReason {
    AmbiguousClock,
    ConditionUnavailable,
    DuplicateSummary,
    SourceExpressionUnsupported,
    SupportingTableNoIndependentTrigger,
    VisualStateRequired,
}
