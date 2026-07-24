use image::{Rgb, RgbImage};
use sc2_copilot_vision::{
    MinimapPingRecognizer, NormalizedRect, PingFrame, PingObservation, UnavailableReason,
};

#[test]
fn a_single_ping_frame_with_a_disconnected_center_core_is_only_a_candidate() {
    let frame = diamond_ping_frame(64, 64, 32, 30, 9);
    let observation =
        MinimapPingRecognizer::default().observe(PingFrame::available("session-1", 1, &frame));

    assert!(matches!(
        observation,
        PingObservation::Candidate {
            core_bounds: NormalizedRect {
                left: 0.46875,
                top: 0.4375,
                right: 0.546875,
                bottom: 0.515625,
            },
            ..
        }
    ));
}

#[test]
fn an_expanding_ping_is_confirmed_on_the_second_frame() {
    let first = diamond_ping_frame(64, 64, 32, 30, 9);
    let second = diamond_ping_frame(64, 64, 32, 30, 11);
    let mut recognizer = MinimapPingRecognizer::default();

    assert!(matches!(
        recognizer.observe(PingFrame::available("session-1", 1, &first)),
        PingObservation::Candidate { .. }
    ));
    assert!(matches!(
        recognizer.observe(PingFrame::available("session-1", 2, &second)),
        PingObservation::Confirmed { .. }
    ));
}

#[test]
fn dropped_source_frames_do_not_break_an_observed_animation() {
    let first = diamond_ping_frame(64, 64, 32, 30, 9);
    let second = diamond_ping_frame(64, 64, 32, 30, 11);
    let mut recognizer = MinimapPingRecognizer::default();

    recognizer.observe(PingFrame::available("session-1", 1, &first));

    assert!(matches!(
        recognizer.observe(PingFrame::available("session-1", 3, &second)),
        PingObservation::Confirmed { .. }
    ));
}

#[test]
fn a_confirmed_ping_stays_confirmed_while_it_remains_visible() {
    let first = diamond_ping_frame(64, 64, 32, 30, 9);
    let expanded = diamond_ping_frame(64, 64, 32, 30, 11);
    let mut recognizer = MinimapPingRecognizer::default();

    recognizer.observe(PingFrame::available("session-1", 1, &first));
    recognizer.observe(PingFrame::available("session-1", 2, &expanded));

    assert!(matches!(
        recognizer.observe(PingFrame::available("session-1", 3, &expanded)),
        PingObservation::Confirmed { .. }
    ));
}

#[test]
fn an_empty_roi_is_reported_as_unavailable() {
    let frame = RgbImage::new(0, 0);
    let observation =
        MinimapPingRecognizer::default().observe(PingFrame::available("session-1", 1, &frame));

    assert_eq!(
        observation,
        PingObservation::Unavailable {
            reason: UnavailableReason::UnsupportedLayout,
        }
    );
}

#[test]
fn a_static_red_diamond_never_becomes_confirmed() {
    let frame = diamond_ping_frame(64, 64, 32, 30, 9);
    let mut recognizer = MinimapPingRecognizer::default();

    recognizer.observe(PingFrame::available("session-1", 1, &frame));

    assert!(matches!(
        recognizer.observe(PingFrame::available("session-1", 2, &frame)),
        PingObservation::Candidate { .. }
    ));
}

#[test]
fn solid_red_objects_and_scattered_noise_are_not_ping_candidates() {
    let mut solid = RgbImage::from_pixel(64, 64, Rgb([18, 24, 30]));
    for y in 20..36 {
        for x in 24..40 {
            solid.put_pixel(x, y, Rgb([242, 24, 31]));
        }
    }
    let mut noise = RgbImage::from_pixel(64, 64, Rgb([18, 24, 30]));
    for (x, y) in [(4, 8), (13, 51), (29, 17), (47, 42), (58, 6)] {
        noise.put_pixel(x, y, Rgb([242, 24, 31]));
    }

    assert_eq!(
        MinimapPingRecognizer::default().observe(PingFrame::available("session-1", 1, &solid)),
        PingObservation::NoEvidence
    );
    assert_eq!(
        MinimapPingRecognizer::default().observe(PingFrame::available("session-1", 1, &noise)),
        PingObservation::NoEvidence
    );
}

#[test]
fn an_outer_ring_without_a_center_core_is_not_a_ping_candidate() {
    let frame = diamond_ring_frame(64, 64, 32, 30, 9);

    assert_eq!(
        MinimapPingRecognizer::default().observe(PingFrame::available("session-1", 1, &frame)),
        PingObservation::NoEvidence
    );
}

#[test]
fn expanding_red_circles_are_not_ping_candidates() {
    let first = circle_frame(64, 64, 32, 30, 9);
    let second = circle_frame(64, 64, 32, 30, 11);
    let mut recognizer = MinimapPingRecognizer::default();

    assert_eq!(
        recognizer.observe(PingFrame::available("session-1", 1, &first)),
        PingObservation::NoEvidence
    );
    assert_eq!(
        recognizer.observe(PingFrame::available("session-1", 2, &second)),
        PingObservation::NoEvidence
    );
}

#[test]
fn a_high_saturation_purple_effect_is_not_a_red_ping() {
    let frame = diamond_frame_with_color(64, 64, 32, 30, 9, Rgb([220, 20, 180]));

    assert_eq!(
        MinimapPingRecognizer::default().observe(PingFrame::available("session-1", 1, &frame)),
        PingObservation::NoEvidence
    );
}

#[test]
fn a_new_session_does_not_inherit_an_old_candidate() {
    let first = diamond_ping_frame(64, 64, 32, 30, 9);
    let expanded = diamond_ping_frame(64, 64, 32, 30, 11);
    let mut recognizer = MinimapPingRecognizer::default();

    recognizer.observe(PingFrame::available("session-1", 1, &first));

    assert!(matches!(
        recognizer.observe(PingFrame::available("session-2", 2, &expanded)),
        PingObservation::Candidate { .. }
    ));
}

#[test]
fn an_unavailable_frame_clears_the_pending_candidate() {
    let first = diamond_ping_frame(64, 64, 32, 30, 9);
    let expanded = diamond_ping_frame(64, 64, 32, 30, 11);
    let mut recognizer = MinimapPingRecognizer::default();

    recognizer.observe(PingFrame::available("session-1", 1, &first));
    assert_eq!(
        recognizer.observe(PingFrame::unavailable(
            "session-1",
            2,
            UnavailableReason::CaptureFailed,
        )),
        PingObservation::Unavailable {
            reason: UnavailableReason::CaptureFailed,
        }
    );

    assert!(matches!(
        recognizer.observe(PingFrame::available("session-1", 3, &expanded)),
        PingObservation::Candidate { .. }
    ));
}

fn diamond_ping_frame(
    width: u32,
    height: u32,
    center_x: i32,
    center_y: i32,
    radius: i32,
) -> RgbImage {
    diamond_frame_with_color(
        width,
        height,
        center_x,
        center_y,
        radius,
        Rgb([242, 24, 31]),
    )
}

fn diamond_frame_with_color(
    width: u32,
    height: u32,
    center_x: i32,
    center_y: i32,
    radius: i32,
    color: Rgb<u8>,
) -> RgbImage {
    let mut frame = RgbImage::from_pixel(width, height, Rgb([18, 24, 30]));
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let distance = (x - center_x).abs() + (y - center_y).abs();
            if distance <= 2 || (radius - 1..=radius + 1).contains(&distance) {
                frame.put_pixel(x as u32, y as u32, color);
            }
        }
    }
    frame
}

fn diamond_ring_frame(
    width: u32,
    height: u32,
    center_x: i32,
    center_y: i32,
    radius: i32,
) -> RgbImage {
    let mut frame = RgbImage::from_pixel(width, height, Rgb([18, 24, 30]));
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let distance = (x - center_x).abs() + (y - center_y).abs();
            if (radius - 1..=radius + 1).contains(&distance) {
                frame.put_pixel(x as u32, y as u32, Rgb([242, 24, 31]));
            }
        }
    }
    frame
}

fn circle_frame(width: u32, height: u32, center_x: i32, center_y: i32, radius: i32) -> RgbImage {
    let mut frame = RgbImage::from_pixel(width, height, Rgb([18, 24, 30]));
    let inner_radius_squared = (radius - 1).pow(2);
    let outer_radius_squared = (radius + 1).pow(2);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let distance_squared = (x - center_x).pow(2) + (y - center_y).pow(2);
            let center_distance = (x - center_x).abs() + (y - center_y).abs();
            if center_distance <= 2
                || (inner_radius_squared..=outer_radius_squared).contains(&distance_squared)
            {
                frame.put_pixel(x as u32, y as u32, Rgb([242, 24, 31]));
            }
        }
    }
    frame
}
