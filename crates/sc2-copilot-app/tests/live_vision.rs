use sc2_copilot_app::{MapVariantVision, VisionContext};
use sc2_copilot_vision::{
    NormalizedPoint, NormalizedRect, PingObservation, UnavailableReason, VisionUpdate,
};

#[test]
fn a_confirmed_lower_left_temple_ping_selects_layout_b() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            195_000,
        ))),
        None
    );

    let update = vision.observe_ping(confirmed_ping(0.25, 0.75, 0.23, 0.73, 0.27, 0.77));

    assert_eq!(
        update,
        Some(VisionUpdate::map_variant(
            "session-1",
            "temple-of-the-past",
            "layout-b",
        ))
    );
}

#[test]
fn a_confirmed_void_rifts_ping_with_its_core_inside_the_region_selects_layout_a() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(
            VisionContext::new("session-1", "void-rifts", 180_000,)
        )),
        None
    );

    let update = vision.observe_ping(confirmed_ping(0.62, 0.45, 0.60, 0.42, 0.64, 0.48));

    assert_eq!(
        update,
        Some(VisionUpdate::map_variant(
            "session-1",
            "void-rifts",
            "layout-a",
        ))
    );
}

#[test]
fn valid_no_evidence_through_the_temple_window_selects_layout_a_after_it_ends() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            195_000,
        ))),
        None
    );
    assert_eq!(vision.observe_ping(PingObservation::NoEvidence), None);

    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            200_001,
        ))),
        Some(VisionUpdate::map_variant(
            "session-1",
            "temple-of-the-past",
            "layout-a",
        ))
    );
}

#[test]
fn unavailable_frames_do_not_trigger_an_absent_ping_decision() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            195_000,
        ))),
        None
    );
    assert_eq!(
        vision.observe_ping(PingObservation::Unavailable {
            reason: UnavailableReason::CaptureFailed,
        }),
        None
    );

    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            200_001,
        ))),
        None
    );
}

#[test]
fn a_new_session_does_not_inherit_an_old_valid_frame() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-1",
            "temple-of-the-past",
            195_000,
        ))),
        None
    );
    assert_eq!(vision.observe_ping(PingObservation::NoEvidence), None);

    assert_eq!(
        vision.update_context(Some(VisionContext::new(
            "session-2",
            "temple-of-the-past",
            200_001,
        ))),
        None
    );
}

#[test]
fn first_observation_after_the_window_does_not_guess_a_variant() {
    let mut vision = MapVariantVision::default();

    assert_eq!(
        vision.update_context(Some(
            VisionContext::new("session-1", "void-rifts", 190_001,)
        )),
        None
    );
    assert_eq!(vision.observe_ping(PingObservation::NoEvidence), None);
}

#[test]
fn valid_no_evidence_through_the_void_rifts_window_selects_layout_b() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(
            VisionContext::new("session-1", "void-rifts", 180_000,)
        )),
        None
    );
    assert_eq!(vision.observe_ping(PingObservation::NoEvidence), None);

    assert_eq!(
        vision.update_context(Some(
            VisionContext::new("session-1", "void-rifts", 190_001,)
        )),
        Some(VisionUpdate::map_variant(
            "session-1",
            "void-rifts",
            "layout-b",
        ))
    );
}

#[test]
fn void_rifts_requires_the_entire_ping_core_inside_its_region() {
    let mut vision = MapVariantVision::default();
    assert_eq!(
        vision.update_context(Some(
            VisionContext::new("session-1", "void-rifts", 180_000,)
        )),
        None
    );

    assert_eq!(
        vision.observe_ping(confirmed_ping(0.60, 0.45, 0.58, 0.42, 0.64, 0.48)),
        None
    );
}

fn confirmed_ping(x: f32, y: f32, left: f32, top: f32, right: f32, bottom: f32) -> PingObservation {
    PingObservation::Confirmed {
        position: NormalizedPoint { x, y },
        core_bounds: NormalizedRect {
            left,
            top,
            right,
            bottom,
        },
        confidence: 0.9,
    }
}
