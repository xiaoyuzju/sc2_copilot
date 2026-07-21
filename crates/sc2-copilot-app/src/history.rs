use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use thiserror::Error;

use crate::{AlertCard, AppController, ConnectionState};

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryKey {
    state: &'static str,
    session_id: Option<String>,
    detected_map_id: Option<String>,
    selected_map_id: Option<String>,
    game_time_second: Option<u64>,
    selected_variant_id: Option<String>,
    active_mutator_ids: Vec<String>,
    stage_anchor_ids: Vec<String>,
    upcoming_event_ids: Vec<String>,
    diagnostics: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SessionHistoryRecord {
    recorded_at_unix_milliseconds: u128,
    state: &'static str,
    session_id: Option<String>,
    detected_map_id: Option<String>,
    selected_map_id: Option<String>,
    game_time_milliseconds: Option<u64>,
    selected_variant_id: Option<String>,
    active_mutator_ids: Vec<String>,
    stage_anchor_ids: Vec<String>,
    upcoming_event_count: usize,
    upcoming_event_ids: Vec<String>,
    alert_event_ids: Vec<Vec<String>>,
    alert_event_labels: Vec<Vec<String>>,
    diagnostics: Vec<String>,
}

pub struct SessionHistory {
    path: PathBuf,
    writer: BufWriter<File>,
    previous: Option<HistoryKey>,
}

impl SessionHistory {
    pub fn new(path: PathBuf) -> Result<Self, HistoryError> {
        let parent = path.parent().ok_or(HistoryError::MissingParent)?;
        fs::create_dir_all(parent)?;
        let writer = BufWriter::new(OpenOptions::new().create(true).append(true).open(&path)?);
        Ok(Self {
            path,
            writer,
            previous: None,
        })
    }

    pub fn for_current_user() -> Result<Self, HistoryError> {
        let app_data = std::env::var_os("APPDATA").ok_or(HistoryError::MissingAppData)?;
        Self::new(
            PathBuf::from(app_data)
                .join("SC2 Copilot")
                .join("history.jsonl"),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(
        &mut self,
        controller: &AppController,
        alerts: &[AlertCard],
    ) -> Result<bool, HistoryError> {
        let view = controller.view();
        let state = match controller.connection() {
            ConnectionState::Disconnected => "disconnected",
            ConnectionState::Menu => "menu",
            ConnectionState::InGame => "in_game",
        };
        let in_game = controller.connection() == ConnectionState::InGame;
        let session_id = in_game
            .then(|| controller.observed_session_id().map(str::to_owned))
            .flatten();
        let detected_map_id = in_game
            .then(|| controller.auto_map_id().map(str::to_owned))
            .flatten();
        let selected_map_id = in_game
            .then(|| controller.selected_map_id().map(str::to_owned))
            .flatten();
        let game_time_milliseconds = in_game
            .then_some(controller.observed_game_time_milliseconds())
            .flatten();
        let engine_view_is_current =
            in_game && view.session_id == session_id && view.map_id == selected_map_id;
        let diagnostics = controller.diagnostics().iter().cloned().collect::<Vec<_>>();
        let selected_variant_id = engine_view_is_current
            .then(|| view.variant_id.clone())
            .flatten();
        let active_mutator_ids = engine_view_is_current
            .then(|| view.active_mutator_ids.clone())
            .unwrap_or_default();
        let stage_anchor_ids = engine_view_is_current
            .then(|| {
                view.stage_anchors
                    .iter()
                    .map(|anchor| anchor.stage_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let upcoming_event_ids = engine_view_is_current
            .then(|| {
                view.upcoming_events
                    .iter()
                    .map(|event| event.event_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let key = HistoryKey {
            state,
            session_id: session_id.clone(),
            detected_map_id: detected_map_id.clone(),
            selected_map_id: selected_map_id.clone(),
            game_time_second: game_time_milliseconds.map(|milliseconds| milliseconds / 1_000),
            selected_variant_id: selected_variant_id.clone(),
            active_mutator_ids: active_mutator_ids.clone(),
            stage_anchor_ids: stage_anchor_ids.clone(),
            upcoming_event_ids: upcoming_event_ids.clone(),
            diagnostics: diagnostics.clone(),
        };
        if alerts.is_empty() && self.previous.as_ref() == Some(&key) {
            return Ok(false);
        }

        let record = SessionHistoryRecord {
            recorded_at_unix_milliseconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            state,
            session_id,
            detected_map_id,
            selected_map_id,
            game_time_milliseconds,
            selected_variant_id,
            active_mutator_ids,
            stage_anchor_ids,
            upcoming_event_count: upcoming_event_ids.len(),
            upcoming_event_ids,
            alert_event_ids: alerts.iter().map(|alert| alert.event_ids.clone()).collect(),
            alert_event_labels: alerts
                .iter()
                .map(|alert| alert.event_labels.clone())
                .collect(),
            diagnostics,
        };
        serde_json::to_writer(&mut self.writer, &record)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        self.previous = Some(key);
        Ok(true)
    }
}

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("APPDATA is unavailable")]
    MissingAppData,
    #[error("history path has no parent directory")]
    MissingParent,
    #[error("history I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("history JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
