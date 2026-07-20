use std::collections::HashSet;

use serde::Deserialize;
use thiserror::Error;

use crate::MissionSchedule;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub struct ScheduleCatalog {
    maps: Vec<MissionSchedule>,
}

impl ScheduleCatalog {
    pub fn from_json(bytes: &[u8]) -> Result<Self, CatalogError> {
        let document: CatalogDocument = serde_json::from_slice(bytes)?;
        if document.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(CatalogError::UnsupportedSchemaVersion(
                document.schema_version,
            ));
        }

        for map in &document.maps {
            let mut event_ids = HashSet::new();
            for event in map.events() {
                if event.map_id() != map.id() {
                    return Err(CatalogError::EventMapMismatch {
                        schedule_map_id: map.id().to_owned(),
                        event_map_id: event.map_id().to_owned(),
                        event_id: event.id().to_owned(),
                    });
                }
                if event.source_refs().is_empty() {
                    return Err(CatalogError::MissingSourceRef {
                        map_id: map.id().to_owned(),
                        event_id: event.id().to_owned(),
                    });
                }
                if event.source_refs().iter().any(|source_ref| {
                    source_ref.source_url.trim().is_empty()
                        || source_ref.snapshot_batch.trim().is_empty()
                        || source_ref.snapshot_path.trim().is_empty()
                }) {
                    return Err(CatalogError::InvalidSourceRef {
                        map_id: map.id().to_owned(),
                        event_id: event.id().to_owned(),
                    });
                }
                if !event_ids.insert(event.id()) {
                    return Err(CatalogError::DuplicateEventId {
                        map_id: map.id().to_owned(),
                        event_id: event.id().to_owned(),
                    });
                }
            }
        }

        Ok(Self {
            maps: document.maps,
        })
    }

    pub fn map_count(&self) -> usize {
        self.maps.len()
    }

    pub fn schedules(&self) -> &[MissionSchedule] {
        &self.maps
    }

    pub fn schedule_for(&self, map_id: &str, variant_id: Option<&str>) -> Option<&MissionSchedule> {
        if variant_id.is_some() {
            return None;
        }

        self.maps.iter().find(|map| map.id() == map_id)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogDocument {
    schema_version: u32,
    maps: Vec<MissionSchedule>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("invalid schedule catalog JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unsupported schedule catalog schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("map {map_id} contains duplicate event ID {event_id}")]
    DuplicateEventId { map_id: String, event_id: String },
    #[error("event {event_id} in map {map_id} has no source reference")]
    MissingSourceRef { map_id: String, event_id: String },
    #[error("event {event_id} in map {map_id} has an invalid source reference")]
    InvalidSourceRef { map_id: String, event_id: String },
    #[error("event {event_id} belongs to map {event_map_id}, not containing map {schedule_map_id}")]
    EventMapMismatch {
        schedule_map_id: String,
        event_map_id: String,
        event_id: String,
    },
}
