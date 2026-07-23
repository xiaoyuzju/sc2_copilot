use std::collections::BTreeMap;

use image::{GrayImage, ImageBuffer, Luma, Rgb, RgbImage};
use imageproc::{
    distance_transform::Norm,
    morphology::close,
    region_labelling::{Connectivity, connected_components},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    CaptureFailed,
    UnsupportedLayout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PingObservation {
    Unavailable {
        reason: UnavailableReason,
    },
    NoEvidence,
    Candidate {
        position: NormalizedPoint,
        confidence: f32,
    },
    Confirmed {
        position: NormalizedPoint,
        confidence: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionUpdate {
    pub session_id: String,
    pub map_id: String,
    pub evidence: VisionEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisionEvidence {
    MapVariant { variant_id: String },
}

impl VisionUpdate {
    pub fn map_variant(
        session_id: impl Into<String>,
        map_id: impl Into<String>,
        variant_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            map_id: map_id.into(),
            evidence: VisionEvidence::MapVariant {
                variant_id: variant_id.into(),
            },
        }
    }
}

pub enum PingFrame<'a> {
    Available {
        session_id: &'a str,
        frame_id: u64,
        roi: &'a RgbImage,
    },
    Unavailable {
        session_id: &'a str,
        frame_id: u64,
        reason: UnavailableReason,
    },
}

impl<'a> PingFrame<'a> {
    pub fn available(session_id: &'a str, frame_id: u64, roi: &'a RgbImage) -> Self {
        Self::Available {
            session_id,
            frame_id,
            roi,
        }
    }

    pub fn unavailable(session_id: &'a str, frame_id: u64, reason: UnavailableReason) -> Self {
        Self::Unavailable {
            session_id,
            frame_id,
            reason,
        }
    }
}

#[derive(Default)]
pub struct MinimapPingRecognizer {
    tracked: Option<TrackedCandidate>,
}

impl MinimapPingRecognizer {
    pub fn observe(&mut self, frame: PingFrame<'_>) -> PingObservation {
        match frame {
            PingFrame::Available {
                session_id,
                frame_id,
                roi,
            } => {
                if roi.width() == 0 || roi.height() == 0 {
                    self.tracked = None;
                    return PingObservation::Unavailable {
                        reason: UnavailableReason::UnsupportedLayout,
                    };
                }
                let Some(candidate) = detect_candidate(roi) else {
                    self.tracked = None;
                    return PingObservation::NoEvidence;
                };
                let confirmed = self.tracked.as_ref().is_some_and(|tracked| {
                    let same_track = tracked.session_id == session_id
                        && frame_id > tracked.frame_id
                        && tracked.candidate.is_same_location(candidate);
                    same_track
                        && (tracked.confirmed
                            || tracked.candidate.is_different_animation_phase(candidate))
                });
                self.tracked = Some(TrackedCandidate {
                    session_id: session_id.to_owned(),
                    frame_id,
                    candidate,
                    confirmed,
                });
                if confirmed {
                    PingObservation::Confirmed {
                        position: candidate.position,
                        confidence: candidate.confidence,
                    }
                } else {
                    PingObservation::Candidate {
                        position: candidate.position,
                        confidence: candidate.confidence,
                    }
                }
            }
            PingFrame::Unavailable { reason, .. } => {
                self.tracked = None;
                PingObservation::Unavailable { reason }
            }
        }
    }
}

struct TrackedCandidate {
    session_id: String,
    frame_id: u64,
    candidate: Candidate,
    confirmed: bool,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    position: NormalizedPoint,
    confidence: f32,
    pixel_count: u32,
    extent: u32,
}

impl Candidate {
    fn is_same_location(self, other: Self) -> bool {
        let delta_x = self.position.x - other.position.x;
        let delta_y = self.position.y - other.position.y;
        delta_x.mul_add(delta_x, delta_y * delta_y) <= 0.06_f32.powi(2)
    }

    fn is_different_animation_phase(self, other: Self) -> bool {
        self.extent.abs_diff(other.extent) >= 2
            || self.pixel_count.abs_diff(other.pixel_count)
                >= self.pixel_count.min(other.pixel_count) / 8
    }
}

#[derive(Debug)]
struct ComponentStats {
    pixel_count: u32,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

impl ComponentStats {
    fn new(x: u32, y: u32) -> Self {
        Self {
            pixel_count: 0,
            min_x: x,
            min_y: y,
            max_x: x,
            max_y: y,
        }
    }

    fn include(&mut self, x: u32, y: u32) {
        self.pixel_count += 1;
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    fn center_x(&self) -> f32 {
        (self.min_x + self.max_x) as f32 / 2.0
    }

    fn center_y(&self) -> f32 {
        (self.min_y + self.max_y) as f32 / 2.0
    }
}

fn detect_candidate(roi: &RgbImage) -> Option<Candidate> {
    let red_mask = GrayImage::from_fn(roi.width(), roi.height(), |x, y| {
        Luma([if is_red(*roi.get_pixel(x, y)) { 255 } else { 0 }])
    });
    let closed_mask = close(&red_mask, Norm::L1, 1);
    let labels = connected_components(&closed_mask, Connectivity::Eight, Luma([0]));
    let mut components = BTreeMap::<u32, ComponentStats>::new();

    for (x, y, label) in labels.enumerate_pixels() {
        let label = label[0];
        if label != 0 {
            components
                .entry(label)
                .or_insert_with(|| ComponentStats::new(x, y))
                .include(x, y);
        }
    }

    components
        .iter()
        .filter_map(|(&label, component)| score_component(&labels, label, component, &components))
        .max_by(|left, right| left.confidence.total_cmp(&right.confidence))
}

fn is_red(pixel: Rgb<u8>) -> bool {
    let [red, green, blue] = pixel.0;
    let red_value = red as f32;
    let green_value = green as f32;
    let blue_value = blue as f32;
    let max = red_value.max(green_value).max(blue_value);
    let min = red_value.min(green_value).min(blue_value);
    let delta = max - min;
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    let hue = if delta == 0.0 {
        0.0
    } else if max == red_value {
        (60.0 * (green_value - blue_value) / delta).rem_euclid(360.0)
    } else if max == green_value {
        60.0 * ((blue_value - red_value) / delta + 2.0)
    } else {
        60.0 * ((red_value - green_value) / delta + 4.0)
    };

    red >= 140
        && saturation >= 0.55
        && (hue <= 18.0 || hue >= 342.0)
        && red.saturating_sub(green) >= 45
        && red.saturating_sub(blue) >= 35
}

fn score_component(
    labels: &ImageBuffer<Luma<u32>, Vec<u32>>,
    label: u32,
    component: &ComponentStats,
    components: &BTreeMap<u32, ComponentStats>,
) -> Option<Candidate> {
    let width = component.max_x - component.min_x + 1;
    let height = component.max_y - component.min_y + 1;
    if component.pixel_count < 20
        || component.pixel_count > 500
        || width < 9
        || height < 9
        || width > labels.width() / 2
        || height > labels.height() / 2
    {
        return None;
    }

    let aspect_ratio = width as f32 / height as f32;
    let fill_ratio = component.pixel_count as f32 / (width * height) as f32;
    if !(0.7..=1.3).contains(&aspect_ratio) || !(0.12..=0.55).contains(&fill_ratio) {
        return None;
    }

    let center_x = (component.min_x + component.max_x) as f32 / 2.0;
    let center_y = (component.min_y + component.max_y) as f32 / 2.0;
    let core = find_center_core(label, component, components)?;

    let half_width = width as f32 / 2.0;
    let half_height = height as f32 / 2.0;
    let mut diamond_score = 0.0;
    let mut min_edge_distance = f32::INFINITY;
    let mut max_edge_distance = f32::NEG_INFINITY;
    for y in component.min_y..=component.max_y {
        for x in component.min_x..=component.max_x {
            if labels.get_pixel(x, y)[0] != label {
                continue;
            }
            let edge_distance = (x as f32 - center_x).abs() / half_width
                + (y as f32 - center_y).abs() / half_height;
            diamond_score += 1.0 - (edge_distance - 1.0).abs().min(1.0);
            min_edge_distance = min_edge_distance.min(edge_distance);
            max_edge_distance = max_edge_distance.max(edge_distance);
        }
    }
    diamond_score /= component.pixel_count as f32;
    let edge_distance_spread = max_edge_distance - min_edge_distance;
    if diamond_score < 0.65 || edge_distance_spread > 0.35 {
        return None;
    }

    Some(Candidate {
        position: NormalizedPoint {
            x: (core.center_x() + 0.5) / labels.width() as f32,
            y: (core.center_y() + 0.5) / labels.height() as f32,
        },
        confidence: diamond_score,
        pixel_count: component.pixel_count,
        extent: width.max(height),
    })
}

fn find_center_core<'a>(
    outer_label: u32,
    outer: &ComponentStats,
    components: &'a BTreeMap<u32, ComponentStats>,
) -> Option<&'a ComponentStats> {
    let outer_width = outer.max_x - outer.min_x + 1;
    let outer_height = outer.max_y - outer.min_y + 1;
    let outer_center_x = outer.center_x();
    let outer_center_y = outer.center_y();

    components
        .iter()
        .filter(|(label, _)| **label != outer_label)
        .map(|(_, component)| component)
        .filter(|component| {
            let width = component.max_x - component.min_x + 1;
            let height = component.max_y - component.min_y + 1;
            let aspect_ratio = width as f32 / height as f32;
            let fill_ratio = component.pixel_count as f32 / (width * height) as f32;
            width >= 3
                && height >= 3
                && width <= outer_width / 2
                && height <= outer_height / 2
                && (0.65..=1.55).contains(&aspect_ratio)
                && fill_ratio >= 0.25
                && (component.center_x() - outer_center_x).abs() <= outer_width as f32 * 0.15
                && (component.center_y() - outer_center_y).abs() <= outer_height as f32 * 0.15
        })
        .max_by_key(|component| component.pixel_count)
}
