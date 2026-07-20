//! Deterministic domain logic for SC2 Copilot.

mod catalog;
mod model;

pub use catalog::{CatalogError, ScheduleCatalog};
pub use model::{
    CompiledEvent, EventCategory, Fact, LocationSpec, MissionSchedule, RuntimeSupport, SourceRef,
    Trigger, UnitKind, WeightedLocation,
};
