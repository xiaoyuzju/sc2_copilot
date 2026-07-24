use sc2_copilot_vision::{NormalizedPoint, NormalizedRect, PingObservation, VisionUpdate};

const TEMPLE_OF_THE_PAST_MAP_ID: &str = "temple-of-the-past";
const VOID_RIFTS_MAP_ID: &str = "void-rifts";

struct MapRule {
    window_start_ms: u64,
    window_end_ms: u64,
    present_variant_id: &'static str,
    absent_variant_id: &'static str,
    region: DetectionRegion,
}

enum DetectionRegion {
    CenterIn(NormalizedRect),
    CoreBoundsIn(NormalizedRect),
}

const TEMPLE_RULE: MapRule = MapRule {
    window_start_ms: 195_000,
    window_end_ms: 200_000,
    present_variant_id: "layout-b",
    absent_variant_id: "layout-a",
    region: DetectionRegion::CenterIn(NormalizedRect {
        left: 0.0,
        top: 130.0 / 259.0,
        right: 132.0 / 264.0,
        bottom: 1.0,
    }),
};

const VOID_RIFTS_RULE: MapRule = MapRule {
    window_start_ms: 180_000,
    window_end_ms: 190_000,
    present_variant_id: "layout-a",
    absent_variant_id: "layout-b",
    region: DetectionRegion::CoreBoundsIn(NormalizedRect {
        left: 156.0 / 264.0,
        top: 94.0 / 259.0,
        right: 262.0 / 264.0,
        bottom: 189.0 / 259.0,
    }),
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionContext {
    session_id: String,
    map_id: String,
    game_time_ms: u64,
}

impl VisionContext {
    pub fn new(
        session_id: impl Into<String>,
        map_id: impl Into<String>,
        game_time_ms: u64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            map_id: map_id.into(),
            game_time_ms,
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisionPhase {
    Idle,
    Waiting,
    Capturing,
    Resolved,
    Missed,
}

#[derive(Default)]
pub struct MapVariantVision {
    context: Option<VisionContext>,
    state: WindowState,
}

#[derive(Default)]
struct WindowState {
    session_id: String,
    map_id: String,
    saw_window: bool,
    valid_frame_seen: bool,
    resolved: bool,
}

impl MapVariantVision {
    pub fn update_context(&mut self, context: Option<VisionContext>) -> Option<VisionUpdate> {
        let Some(context) = context else {
            self.context = None;
            self.state = WindowState::default();
            return None;
        };

        let changed =
            self.state.session_id != context.session_id || self.state.map_id != context.map_id;
        if changed {
            self.state = WindowState {
                session_id: context.session_id.clone(),
                map_id: context.map_id.clone(),
                ..WindowState::default()
            };
        }

        let rule = rule_for_map(&context.map_id);
        if let Some(rule) = rule
            && (rule.window_start_ms..=rule.window_end_ms).contains(&context.game_time_ms)
        {
            self.state.saw_window = true;
        }
        self.context = Some(context);

        let context = self.context.as_ref().expect("context was just stored");
        let rule = rule?;
        if !self.state.resolved
            && self.state.saw_window
            && self.state.valid_frame_seen
            && context.game_time_ms > rule.window_end_ms
        {
            self.state.resolved = true;
            return Some(VisionUpdate::map_variant(
                &context.session_id,
                &context.map_id,
                rule.absent_variant_id,
            ));
        }

        None
    }

    pub fn observe_ping(&mut self, observation: PingObservation) -> Option<VisionUpdate> {
        let context = self.context.as_ref()?;
        let rule = rule_for_map(&context.map_id)?;
        if self.state.resolved
            || !(rule.window_start_ms..=rule.window_end_ms).contains(&context.game_time_ms)
        {
            return None;
        }

        if !matches!(observation, PingObservation::Unavailable { .. }) {
            self.state.valid_frame_seen = true;
        }

        let PingObservation::Confirmed {
            position,
            core_bounds,
            ..
        } = observation
        else {
            return None;
        };
        if !rule.region.contains(position, core_bounds) {
            return None;
        }

        self.state.resolved = true;
        Some(VisionUpdate::map_variant(
            &context.session_id,
            &context.map_id,
            rule.present_variant_id,
        ))
    }

    pub(crate) fn phase(&self) -> VisionPhase {
        let Some(context) = &self.context else {
            return VisionPhase::Idle;
        };
        let Some(rule) = rule_for_map(&context.map_id) else {
            return VisionPhase::Idle;
        };
        if self.state.resolved {
            VisionPhase::Resolved
        } else if context.game_time_ms < rule.window_start_ms {
            VisionPhase::Waiting
        } else if context.game_time_ms <= rule.window_end_ms {
            VisionPhase::Capturing
        } else {
            VisionPhase::Missed
        }
    }
}

impl DetectionRegion {
    fn contains(&self, position: NormalizedPoint, core_bounds: NormalizedRect) -> bool {
        match self {
            Self::CenterIn(region) => contains_point(*region, position),
            Self::CoreBoundsIn(region) => contains_rect(*region, core_bounds),
        }
    }
}

fn contains_point(region: NormalizedRect, point: NormalizedPoint) -> bool {
    point.x >= region.left
        && point.x <= region.right
        && point.y >= region.top
        && point.y <= region.bottom
}

fn contains_rect(region: NormalizedRect, rect: NormalizedRect) -> bool {
    rect.left >= region.left
        && rect.right <= region.right
        && rect.top >= region.top
        && rect.bottom <= region.bottom
}

fn rule_for_map(map_id: &str) -> Option<&'static MapRule> {
    match map_id {
        TEMPLE_OF_THE_PAST_MAP_ID => Some(&TEMPLE_RULE),
        VOID_RIFTS_MAP_ID => Some(&VOID_RIFTS_RULE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{MapVariantVision, VisionContext, VisionPhase};

    #[test]
    fn only_requests_capture_during_a_target_maps_detection_window() {
        let mut vision = MapVariantVision::default();
        let _ = vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            194_999,
        )));
        assert_eq!(vision.phase(), VisionPhase::Waiting);

        let _ = vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            195_000,
        )));
        assert_eq!(vision.phase(), VisionPhase::Capturing);

        let _ = vision.update_context(Some(VisionContext::new(
            "session-1",
            "dead-of-night",
            195_000,
        )));
        assert_eq!(vision.phase(), VisionPhase::Idle);
    }
}
