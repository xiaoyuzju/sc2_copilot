//! Deterministic domain logic for SC2 Copilot.

mod catalog;
mod engine;
mod model;

pub use catalog::{CatalogError, ScheduleCatalog};
pub use engine::{
    AlertBatch, CopilotEngine, EngineInput, EngineSettings, EngineUpdate, EngineView,
    GameObservation, ScheduledEventView, StageAnchorView, UserCommand,
};
pub use model::{
    CompiledEvent, EventCategory, Fact, LocationSpec, MissionSchedule, RuntimeSupport,
    ScheduleVariant, SourceRef, TableCoverage, Trigger, UnitKind, UnsupportedReason,
    UnsupportedRow, WeightedLocation,
};
