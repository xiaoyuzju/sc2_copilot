use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use sc2_copilot_vision::{
    MinimapPingRecognizer, PingFrame, PingObservation, UnavailableReason, VisionUpdate,
};

use crate::{
    capture::{CaptureError, Sc2MinimapCapture},
    vision::{MapVariantVision, VisionContext, VisionPhase},
};

#[derive(Debug, Clone, Default)]
pub(crate) enum VisionRuntimeState {
    #[default]
    Idle,
    Paused(String),
    Active(VisionContext),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisionSnapshot {
    pub(crate) update: Option<VisionUpdate>,
    pub(crate) status: String,
}

impl Default for VisionSnapshot {
    fn default() -> Self {
        Self {
            update: None,
            status: "等待对局".to_owned(),
        }
    }
}

#[derive(Clone, Default)]
struct LatestVision {
    inner: Arc<Mutex<VisionSnapshot>>,
}

impl LatestVision {
    fn publish(&self, status: impl Into<String>, update: Option<VisionUpdate>) {
        let mut latest = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        latest.status = status.into();
        if update.is_some() {
            latest.update = update;
        }
    }

    fn take(&self) -> VisionSnapshot {
        let mut latest = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        VisionSnapshot {
            update: latest.update.take(),
            status: latest.status.clone(),
        }
    }
}

pub(crate) struct VisionRuntime {
    state: Arc<Mutex<VisionRuntimeState>>,
    latest: LatestVision,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl VisionRuntime {
    pub(crate) fn spawn(interval: Duration) -> Self {
        let state = Arc::new(Mutex::new(VisionRuntimeState::Idle));
        let worker_state = Arc::clone(&state);
        let latest = LatestVision::default();
        let worker_latest = latest.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            run_worker(worker_state, worker_latest, worker_stop, interval);
        });
        Self {
            state,
            latest,
            stop,
            worker: Some(worker),
        }
    }

    pub(crate) fn set_state(&self, state: VisionRuntimeState) {
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) = state;
    }

    pub(crate) fn take_latest(&self) -> VisionSnapshot {
        self.latest.take()
    }
}

impl Drop for VisionRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    state: Arc<Mutex<VisionRuntimeState>>,
    latest: LatestVision,
    stop: Arc<AtomicBool>,
    interval: Duration,
) {
    let mut capture = Sc2MinimapCapture::new();
    let mut recognizer = MinimapPingRecognizer::default();
    let mut vision = MapVariantVision::default();
    let mut frame_id = 0_u64;

    while !stop.load(Ordering::Relaxed) {
        let state = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        match state {
            VisionRuntimeState::Idle => {
                let _ = vision.update_context(None);
                recognizer = MinimapPingRecognizer::default();
                latest.publish("等待对局", None);
            }
            VisionRuntimeState::Paused(reason) => {
                recognizer = MinimapPingRecognizer::default();
                latest.publish(format!("已暂停 · {reason}"), None);
            }
            VisionRuntimeState::Active(context) => {
                process_active_context(
                    &mut capture,
                    &mut recognizer,
                    &mut vision,
                    &mut frame_id,
                    &latest,
                    context,
                );
            }
        }
        thread::sleep(interval);
    }
}

fn process_active_context(
    capture: &mut Sc2MinimapCapture,
    recognizer: &mut MinimapPingRecognizer,
    vision: &mut MapVariantVision,
    frame_id: &mut u64,
    latest: &LatestVision,
    context: VisionContext,
) {
    let context_update = vision.update_context(Some(context.clone()));
    match vision.phase() {
        VisionPhase::Idle => latest.publish("当前地图无需红点识别", context_update),
        VisionPhase::Waiting => latest.publish("等待地图红点检测时间窗", context_update),
        VisionPhase::Resolved => latest.publish("已完成当前对局的红点证据判定", context_update),
        VisionPhase::Missed => latest.publish("已错过当前对局的红点检测时间窗", context_update),
        VisionPhase::Capturing => match capture.capture() {
            Ok(Some(roi)) => {
                *frame_id = frame_id.wrapping_add(1);
                let observation =
                    recognizer.observe(PingFrame::available(context.session_id(), *frame_id, &roi));
                let status = observation_status(observation);
                let update = vision.observe_ping(observation);
                if update.is_some() {
                    latest.publish("已确认地图红点证据", update);
                } else {
                    latest.publish(status, None);
                }
            }
            Ok(None) => latest.publish("检测中 · 等待新画面", None),
            Err(error) => {
                *frame_id = frame_id.wrapping_add(1);
                let reason = unavailable_reason(&error);
                let _ = recognizer.observe(PingFrame::unavailable(
                    context.session_id(),
                    *frame_id,
                    reason,
                ));
                let _ = vision.observe_ping(PingObservation::Unavailable { reason });
                latest.publish(capture_error_status(&error), None);
            }
        },
    }
}

fn observation_status(observation: PingObservation) -> &'static str {
    match observation {
        PingObservation::Unavailable { .. } => "检测中 · 画面暂不可用",
        PingObservation::NoEvidence => "检测中 · 未见红点",
        PingObservation::Candidate { .. } => "检测中 · 正在确认红点",
        PingObservation::Confirmed { .. } => "检测中 · 红点不在目标区域",
    }
}

fn unavailable_reason(error: &CaptureError) -> UnavailableReason {
    match error {
        CaptureError::UnsupportedLayout
        | CaptureError::UnsupportedRotation
        | CaptureError::InvalidMinimap => UnavailableReason::UnsupportedLayout,
        _ => UnavailableReason::CaptureFailed,
    }
}

fn capture_error_status(error: &CaptureError) -> String {
    match error {
        CaptureError::WindowNotFound => "检测中 · 等待 SC2 游戏窗口".to_owned(),
        CaptureError::WindowNotForeground => "已暂停 · SC2 不在前台".to_owned(),
        CaptureError::WindowMinimized => "已暂停 · SC2 窗口已最小化".to_owned(),
        _ => format!("视觉识别不可用 · {error}"),
    }
}

#[cfg(test)]
mod tests {
    use sc2_copilot_vision::VisionUpdate;

    use super::LatestVision;

    #[test]
    fn status_refreshes_do_not_discard_an_unconsumed_variant_update() {
        let latest = LatestVision::default();
        let update = VisionUpdate::map_variant("session-1", "temple-of-the-past", "layout-b");
        latest.publish("已识别", Some(update.clone()));
        latest.publish("已完成", None);

        let snapshot = latest.take();
        assert_eq!(snapshot.status, "已完成");
        assert_eq!(snapshot.update, Some(update));
        assert_eq!(latest.take().update, None);
    }
}
