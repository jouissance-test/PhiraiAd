//! Fixed Preview Time's bounded solver and application preference bridge.

use crate::{
    config::{NOTE_FLOW_SPEED_MAX, NOTE_FLOW_SPEED_MIN},
    core::{Chart, Matrix, Note, NoteKind, Object, Point, Resource, Vector},
    fixed_preview_policy::{FallbackReason, PreviewPlan},
    parse::RPE_HEIGHT,
};
use anyhow::Result;
use macroquad::prelude::Texture2D;
use nalgebra::Rotation2;
use std::sync::OnceLock;

pub static PREFERENCE_SAVER: OnceLock<fn(bool, f32) -> Result<()>> = OnceLock::new();

pub fn save_preference(enabled: bool, seconds: f32) -> Result<()> {
    if let Some(save) = PREFERENCE_SAVER.get() {
        save(enabled, seconds)?;
    }
    Ok(())
}

#[inline]
fn cross(a: Point, b: Point, c: Point) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

#[inline]
fn inside_view(point: Point) -> bool {
    point.x.abs() <= 1. && point.y.abs() <= 1.
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    let on_segment = |start: Point, end: Point, point: Point| {
        point.x >= start.x.min(end.x) - 1e-6
            && point.x <= start.x.max(end.x) + 1e-6
            && point.y >= start.y.min(end.y) - 1e-6
            && point.y <= start.y.max(end.y) + 1e-6
    };
    if ab_c.abs() <= 1e-6 && on_segment(a, b, c)
        || ab_d.abs() <= 1e-6 && on_segment(a, b, d)
        || cd_a.abs() <= 1e-6 && on_segment(c, d, a)
        || cd_b.abs() <= 1e-6 && on_segment(c, d, b)
    {
        return true;
    }
    ab_c.is_sign_positive() != ab_d.is_sign_positive() && cd_a.is_sign_positive() != cd_b.is_sign_positive()
}

fn point_in_convex_quad(point: Point, quad: &[Point; 4]) -> bool {
    let mut sign = 0_i8;
    for i in 0..4 {
        let value = cross(quad[i], quad[(i + 1) % 4], point);
        if value.abs() <= 1e-6 {
            continue;
        }
        let current = if value.is_sign_positive() { 1 } else { -1 };
        if sign != 0 && sign != current {
            return false;
        }
        sign = current;
    }
    true
}

/// Exact convex-quad/viewport intersection after all chart transforms.
pub(crate) fn quad_visible(quad: [Point; 4]) -> bool {
    if quad.iter().any(|point| !point.x.is_finite() || !point.y.is_finite()) {
        return false;
    }
    let area = (0..4)
        .map(|i| quad[i].x * quad[(i + 1) % 4].y - quad[(i + 1) % 4].x * quad[i].y)
        .sum::<f32>()
        .abs();
    if area <= 1e-10 {
        return false;
    }
    if quad.iter().copied().any(inside_view) {
        return true;
    }
    let view = [Point::new(-1., -1.), Point::new(1., -1.), Point::new(1., 1.), Point::new(-1., 1.)];
    if view.iter().copied().any(|point| point_in_convex_quad(point, &quad)) {
        return true;
    }
    (0..4).any(|i| (0..4).any(|j| segments_intersect(quad[i], quad[(i + 1) % 4], view[j], view[(j + 1) % 4])))
}

/// Find the last visible multiplier before a Note leaves the viewport.
///
/// The caller supplies the render-equivalent geometry predicate. A missing
/// visible/outside bracket is a normal unsupported-chart fallback.
pub(crate) fn solve_flow(mut visible: impl FnMut(f32) -> bool, min: f32, max: f32) -> Option<f32> {
    if !min.is_finite() || !max.is_finite() || min <= 0. || max <= min || !visible(min) || visible(max) {
        return None;
    }
    let (mut low, mut high) = (min, max);
    for _ in 0..16 {
        let middle = (low + high) * 0.5;
        if visible(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    low.is_finite().then_some(low)
}

// At most 1/240 real second between samples, with a hard per-candidate budget.
// This is a conservative sampled check, not a proof for arbitrary animation
// between samples. Unsupported durations fall back unchanged.
const VALIDATION_HZ: f64 = 240.;
const MAX_VALIDATION_STEPS: usize = 1536;
// If the target-boundary candidate fails, refine the largest safe constant
// flow between the known-safe minimum and failed upper candidate. This remains
// a single constant-flow plan, not a multi-segment dynamic-flow search.
const CONSTANT_FLOW_REFINEMENT_STEPS: usize = 10;
// A Constant plan may inherit fast authored line motion, but must not add a
// materially faster semantic-head spike than the Original trajectory. The
// absolute floor still rejects a newly introduced spike when Original barely
// moves; 1 NDC/real-second absorbs sampling/float noise in the peak comparison.
const CONSTANT_MOTION_ABSOLUTE_LIMIT_NDC_PER_REAL_SECOND: f64 = 24.;
const CONSTANT_MOTION_ORIGINAL_TOLERANCE_NDC_PER_REAL_SECOND: f64 = 1.;

#[derive(Clone, Copy)]
struct PreviewSample {
    anchor: Point,
    visible: bool,
}

fn validation_steps(start: f64, hit: f64, playback_rate: f64) -> Option<usize> {
    let steps = ((hit - start) / playback_rate * VALIDATION_HZ).ceil();
    if !steps.is_finite() || steps < 1. || steps > MAX_VALIDATION_STEPS as f64 {
        None
    } else {
        Some(steps as usize)
    }
}

fn visible_interval(start: f64, hit: f64, steps: usize, mut visible: impl FnMut(f64) -> bool) -> bool {
    (0..=steps).all(|i| {
        // Judgement may remove the head on the hit frame. Validate its left
        // limit instead, including the entire readable approach interval.
        let time = if i == steps {
            hit - (hit - start).min(1e-7)
        } else {
            start + (hit - start) * i as f64 / steps as f64
        };
        visible(time)
    })
}

fn validate_constant_candidate(
    start: f64,
    hit: f64,
    steps: usize,
    playback_rate: f64,
    flow: f32,
    original_flow: f32,
    sample: &mut impl FnMut(f64, f32) -> Option<PreviewSample>,
) -> Result<(), FallbackReason> {
    let mut previous: Option<(f64, Point, Point)> = None;
    let mut failure = FallbackReason::VisibilityGap;
    let accepted = visible_interval(start, hit, steps, |time| {
        let Some(candidate_sample) = sample(time, flow) else { return false };
        if !candidate_sample.visible {
            return false;
        }
        let anchor = candidate_sample.anchor;
        let Some(original_anchor) = sample(time, original_flow).map(|sample| sample.anchor) else {
            return false;
        };
        if let Some((previous_time, previous_anchor, previous_original_anchor)) = previous {
            let real_dt = (time - previous_time) / playback_rate;
            let speed = (anchor - previous_anchor).norm() as f64 / real_dt;
            let original_speed = (original_anchor - previous_original_anchor).norm() as f64 / real_dt;
            let motion_speed_limit =
                CONSTANT_MOTION_ABSOLUTE_LIMIT_NDC_PER_REAL_SECOND.max(original_speed + CONSTANT_MOTION_ORIGINAL_TOLERANCE_NDC_PER_REAL_SECOND);
            if !speed.is_finite() || !original_speed.is_finite() || speed > motion_speed_limit {
                failure = FallbackReason::MotionSpike;
                return false;
            }
        }
        previous = Some((time, anchor, original_anchor));
        true
    });
    accepted.then_some(()).ok_or(failure)
}

fn choose_plan(
    fake: bool,
    original_flow: f32,
    start: f64,
    hit: f64,
    playback_rate: f64,
    mut sample: impl FnMut(f64, f32) -> Option<PreviewSample>,
) -> PreviewPlan {
    let original = |reason| PreviewPlan::original(original_flow, reason);
    if fake {
        return original(FallbackReason::FakeNote);
    }
    if !start.is_finite() || start < 0. || !hit.is_finite() || hit <= start || !playback_rate.is_finite() || playback_rate <= 0. {
        return original(FallbackReason::InvalidTarget);
    }
    let Some(steps) = validation_steps(start, hit, playback_rate) else {
        return original(FallbackReason::ValidationBudget);
    };
    // Preserve authored motion whenever gating alone already supplies a full
    // readable interval. Do not accelerate a reversing trajectory needlessly.
    if visible_interval(start, hit, steps, |time| sample(time, original_flow).is_some_and(|sample| sample.visible)) {
        return PreviewPlan::GateOnly { original_flow, start };
    }
    let Some(flow) = solve_flow(|flow| sample(start, flow).is_some_and(|sample| sample.visible), NOTE_FLOW_SPEED_MIN, NOTE_FLOW_SPEED_MAX) else {
        return original(FallbackReason::NoBracket);
    };
    let boundary_failure = match validate_constant_candidate(start, hit, steps, playback_rate, flow, original_flow, &mut sample) {
        Ok(()) => return PreviewPlan::Constant { flow, start },
        Err(reason) => reason,
    };
    let minimum_failure = match validate_constant_candidate(start, hit, steps, playback_rate, NOTE_FLOW_SPEED_MIN, original_flow, &mut sample) {
        Ok(()) => {
            let (mut low, mut high) = (NOTE_FLOW_SPEED_MIN, flow);
            for _ in 0..CONSTANT_FLOW_REFINEMENT_STEPS {
                let middle = (low + high) * 0.5;
                if validate_constant_candidate(start, hit, steps, playback_rate, middle, original_flow, &mut sample).is_ok() {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return PreviewPlan::Constant { flow: low, start };
        }
        Err(reason) => reason,
    };
    original(if boundary_failure == FallbackReason::MotionSpike || minimum_failure == FallbackReason::MotionSpike {
        FallbackReason::MotionSpike
    } else {
        FallbackReason::VisibilityGap
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RebuildStats {
    pub solved: usize,
    pub gate_only: usize,
    pub fallback: usize,
}

#[inline]
fn sample(anim: &crate::core::AnimFloat, time: f64, default: f32) -> f32 {
    anim.value_at(time).unwrap_or(default)
}

fn object_translation_at(object: &Object, time: f64, aspect_ratio: f32) -> Vector {
    Vector::new(sample(&object.translation.0, time, 0.), sample(&object.translation.1, time, 0.) / aspect_ratio)
}

fn object_scale_at(object: &Object, time: f64) -> Vector {
    Vector::new(sample(&object.scale.0, time, 1.), sample(&object.scale.1, time, 1.))
}

fn line_rotation_at(chart: &Chart, id: usize, time: f64) -> f32 {
    let line = &chart.lines[id];
    let mut rotation = sample(&line.object.rotation, time, 0.);
    if line.rot_with_parent {
        if let Some(parent) = line.parent {
            rotation += line_rotation_at(chart, parent, time);
        }
    }
    rotation
}

fn line_position_at(chart: &Chart, id: usize, time: f64, aspect_ratio: f32) -> Vector {
    let line = &chart.lines[id];
    let own = object_translation_at(&line.object, time, aspect_ratio);
    if let Some(parent) = line.parent {
        line_position_at(chart, parent, time, aspect_ratio) + Rotation2::new(line_rotation_at(chart, parent, time).to_radians()) * own
    } else {
        own
    }
}

fn line_transform_at(chart: &Chart, id: usize, time: f64, aspect_ratio: f32) -> Matrix {
    crate::note_visual::line_transform(line_rotation_at(chart, id, time), line_position_at(chart, id, time, aspect_ratio))
}

fn texture_for(note: &Note, res: &Resource) -> Texture2D {
    let style = if res.config.double_hint && note.multiple_hint {
        &res.res_pack.note_style_mh
    } else {
        &res.res_pack.note_style
    };
    **match note.kind {
        NoteKind::Click => &style.click,
        NoteKind::Flick => &style.flick,
        NoteKind::Drag => &style.drag,
        NoteKind::Hold { .. } => &style.hold,
    }
}

fn local_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> [Point; 4] {
    [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
}

fn preview_ndc(res: &Resource, point: Point) -> Point {
    let width = res.camera.viewport.map_or(res.last_vp.2, |viewport| viewport.2).max(1) as f32;
    let (mut cx, cy) = res.practice_view.center(width);
    if res.config.flip_x() {
        cx = -cx;
    }
    let scale = res.practice_view.scale();
    Point::new((point.x - cx) * scale, (point.y + cy) * scale * res.aspect_ratio)
}

struct NoteGeometry {
    quads: [[Point; 4]; 3],
    anchor: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeometryAbsence {
    InvalidInput,
    ZeroSpeed,
    ControlNonFinite,
    TransformNonFinite,
    OutputNonFinite,
}

impl GeometryAbsence {
    #[cfg(feature = "render-audit")]
    fn name(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::ZeroSpeed => "zero_speed",
            Self::ControlNonFinite => "control_non_finite",
            Self::TransformNonFinite => "transform_non_finite",
            Self::OutputNonFinite => "output_non_finite",
        }
    }
}

fn note_geometry_at_diagnostic(
    chart: &Chart,
    line_id: usize,
    note_id: usize,
    res: &Resource,
    time: f64,
    flow: f32,
) -> Result<NoteGeometry, GeometryAbsence> {
    let line = &chart.lines[line_id];
    let note = &line.notes[note_id];
    if !time.is_finite() || !flow.is_finite() || flow <= 0. || res.aspect_ratio.abs() <= f32::EPSILON {
        return Err(GeometryAbsence::InvalidInput);
    }
    let line_height = sample(&line.height, time, 0.) as f64;
    let y_offset = sample(&note.object.translation.1, time, 0.);
    if note.speed.abs() <= f64::EPSILON {
        return Err(GeometryAbsence::ZeroSpeed);
    }
    let ctrl_height = (note.height - line_height + y_offset as f64 / note.speed) * RPE_HEIGHT as f64 / 2.;
    let ctrl = line.ctrl_obj.borrow();
    let ctrl_y = sample(&ctrl.y, ctrl_height, 1.);
    let ctrl_size = sample(&ctrl.size, ctrl_height, 1.);
    let ctrl_pos = sample(&ctrl.pos, ctrl_height, 1.);
    if [ctrl_y, ctrl_size, ctrl_pos].iter().any(|value| !value.is_finite()) {
        return Err(GeometryAbsence::ControlNonFinite);
    }

    let speed = note.speed * ctrl_y as f64;
    let base = ((note.height - line_height) / res.aspect_ratio as f64 * speed) as f32;
    let translation = object_translation_at(&note.object, time, res.aspect_ratio);
    let note_scale = object_scale_at(&note.object, time);
    if translation.iter().chain(note_scale.iter()).any(|value| !value.is_finite()) {
        return Err(GeometryAbsence::TransformNonFinite);
    }

    let multiple_scale = if res.config.double_hint && note.multiple_hint {
        res.res_pack.note_style_mh.click.width() / res.res_pack.note_style.click.width()
    } else {
        1.
    };
    let half_width = res.note_width * multiple_scale;
    let geometry = crate::note_visual::NoteTransform {
        translation,
        scale: note_scale,
        rotation: sample(&note.object.rotation, time, 0.),
        y_offset,
        ctrl_size,
        ctrl_pos,
        uniform_scale: res.info.note_uniform_scale,
        hold: matches!(note.kind, NoteKind::Hold { .. }),
        incline_sin: sample(&line.incline, time, 0.).to_radians().sin(),
        aspect_ratio: res.aspect_ratio,
    };
    let (note_transform, local, anchor_local) = match note.kind {
        NoteKind::Hold { end_time, end_height } => {
            let end_height = if res.config.shorten_holds && !note.fake {
                let visual_end =
                    crate::chart_play::hold_visual_end(note.time, end_time, res.config.speed as f64, crate::judge::hold_tail_window(&res.config));
                let sampled_height = if visual_end <= note.time {
                    note.height
                } else {
                    sample(&line.height, visual_end, note.height as f32) as f64
                };
                crate::chart_play::hold_render_end_height(note.height, sampled_height, speed / res.aspect_ratio as f64 * flow as f64)
            } else {
                end_height
            };
            let head = base * flow;
            let tail = (((end_height - line_height) / res.aspect_ratio as f64 * speed) as f32) * flow;
            let style = if res.config.double_hint && note.multiple_hint {
                &res.res_pack.note_style_mh
            } else {
                &res.res_pack.note_style
            };
            let ratio = style.hold_ratio();
            let head_half = style.hold_head_rect().h / style.hold_head_rect().w * half_width * ratio;
            let tail_half = style.hold_tail_rect().h / style.hold_tail_rect().w * half_width * ratio;
            let head_y = head - if res.res_pack.info.hold_compact { head_half } else { head_half * 2. };
            let tail_y = tail - if res.res_pack.info.hold_compact { tail_half } else { 0. };
            // Test each actual rendered piece, not the bounding envelope: a
            // reversed body is skipped by draw_tex, and the empty gap between
            // an off-screen head and tail must not count as a visible Hold.
            let body_top = tail.max(head);
            (
                geometry.matrix(0., flow),
                [
                    local_rect(-half_width, head_y, half_width, head_y + head_half * 2.),
                    local_rect(-half_width, head, half_width, body_top),
                    local_rect(-half_width, tail_y, half_width, tail_y + tail_half * 2.),
                ],
                Point::new(0., head),
            )
        }
        _ => {
            let texture = texture_for(note, res);
            let half_height = texture.height() / texture.width() * half_width;
            (geometry.matrix(base, flow), [local_rect(-half_width, -half_height, half_width, half_height); 3], Point::origin())
        }
    };
    // Alpha, negative-alpha appearance modes and cover are renderer-owned
    // visibility gates, not geometry. They remain authored exactly as-is, but
    // must not make an otherwise safe travel trajectory fall back to original
    // speed. The solver only answers whether the transformed quads stay in the
    // viewport after their target boundary contact.
    let (flip_x, flip_y) = crate::chart_play::reflection_axes(res.config.flip_x(), false);
    let mut transform =
        Matrix::identity().append_nonuniform_scaling(&Vector::new(flip_x, -flip_y)) * line_transform_at(chart, line_id, time, res.aspect_ratio);
    if !note.above {
        transform *= Matrix::identity().append_nonuniform_scaling(&Vector::new(1., -1.));
    }
    transform *= note_transform;
    // Semantic Note anchor: Tap center, or Hold head travel position before
    // texture offsets. It cannot jump when body length/topology/skin changes.
    let anchor = preview_ndc(res, transform.transform_point(&anchor_local));
    let quads = local.map(|quad| quad.map(|point| preview_ndc(res, transform.transform_point(&point))));
    if quads
        .iter()
        .flatten()
        .chain(std::iter::once(&anchor))
        .all(|point| point.x.is_finite() && point.y.is_finite())
    {
        Ok(NoteGeometry { quads, anchor })
    } else {
        Err(GeometryAbsence::OutputNonFinite)
    }
}

fn note_geometry_at(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, time: f64, flow: f32) -> Option<NoteGeometry> {
    note_geometry_at_diagnostic(chart, line_id, note_id, res, time, flow).ok()
}

fn note_quad_at(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, time: f64, flow: f32) -> Option<[[Point; 4]; 3]> {
    note_geometry_at(chart, line_id, note_id, res, time, flow).map(|geometry| geometry.quads)
}

fn geometry_visible(quads: [[Point; 4]; 3]) -> bool {
    quads.into_iter().any(quad_visible)
}

#[cfg(feature = "render-audit")]
fn audit_constant_candidate(
    chart: &Chart,
    line_id: usize,
    note_id: usize,
    res: &Resource,
    start: f64,
    hit: f64,
    playback_rate: f64,
    flow: f32,
) -> serde_json::Value {
    let Some(steps) = validation_steps(start, hit, playback_rate) else {
        return serde_json::json!({"flow":flow,"validation_budget":false});
    };
    let mut geometry_missing = 0_usize;
    let mut geometry_missing_reasons = std::collections::BTreeMap::<&'static str, usize>::new();
    let mut outside_viewport = 0_usize;
    let mut visible = 0_usize;
    let mut motion_spike = 0_usize;
    let mut previous: Option<(f64, Point)> = None;
    let mut geometric_previous: Option<(f64, Point)> = None;
    let mut visible_peak_speed = 0_f64;
    let mut geometric_peak_speed = 0_f64;
    for i in 0..=steps {
        let time = if i == steps {
            hit - (hit - start).min(1e-7)
        } else {
            start + (hit - start) * i as f64 / steps as f64
        };
        let geometry = match note_geometry_at_diagnostic(chart, line_id, note_id, res, time, flow) {
            Ok(geometry) => geometry,
            Err(reason) => {
                geometry_missing += 1;
                *geometry_missing_reasons.entry(reason.name()).or_default() += 1;
                previous = None;
                continue;
            }
        };
        if let Some((previous_time, previous_anchor)) = geometric_previous {
            let real_dt = (time - previous_time) / playback_rate;
            let speed = (geometry.anchor - previous_anchor).norm() as f64 / real_dt;
            if speed.is_finite() {
                geometric_peak_speed = geometric_peak_speed.max(speed);
            }
        }
        geometric_previous = Some((time, geometry.anchor));
        if !geometry_visible(geometry.quads) {
            outside_viewport += 1;
            previous = None;
            continue;
        }
        visible += 1;
        if let Some((previous_time, previous_anchor)) = previous {
            let real_dt = (time - previous_time) / playback_rate;
            let speed = (geometry.anchor - previous_anchor).norm() as f64 / real_dt;
            if speed.is_finite() {
                visible_peak_speed = visible_peak_speed.max(speed);
            }
            if !speed.is_finite() || speed > CONSTANT_MOTION_ABSOLUTE_LIMIT_NDC_PER_REAL_SECOND {
                motion_spike += 1;
            }
        }
        previous = Some((time, geometry.anchor));
    }
    serde_json::json!({
        "flow":flow,
        "validation_budget":true,
        "samples":steps+1,
        "geometry_missing":geometry_missing,
        "geometry_missing_reasons":geometry_missing_reasons,
        "outside_viewport":outside_viewport,
        "visible":visible,
        "motion_spike":motion_spike,
        "visible_peak_speed_ndc_per_real_second":visible_peak_speed,
        "geometric_peak_speed_ndc_per_real_second":geometric_peak_speed,
    })
}

#[cfg(feature = "render-audit")]
fn authored_alpha_gate_at(chart: &Chart, line_id: usize, note_id: usize, time: f64) -> Option<&'static str> {
    let line = &chart.lines[line_id];
    let note = &line.notes[note_id];
    let line_alpha = sample(&line.object.alpha, time, 1.);
    if line_alpha < 0. {
        if !chart.settings.pe_alpha_extension {
            return Some("negative_line_alpha_disabled");
        }
        if (-line_alpha).floor() as u32 == 1 {
            return Some("negative_line_alpha_hidden");
        }
    }
    if sample(&note.object.alpha, time, 1.) <= 0. {
        return Some("note_alpha");
    }
    if note.speed.abs() <= f64::EPSILON {
        return None;
    }
    let line_height = sample(&line.height, time, 0.) as f64;
    let y_offset = sample(&note.object.translation.1, time, 0.);
    let ctrl_height = (note.height - line_height + y_offset as f64 / note.speed) * RPE_HEIGHT as f64 / 2.;
    (sample(&line.ctrl_obj.borrow().alpha, ctrl_height, 1.) <= 0.).then_some("control_alpha")
}

#[cfg(feature = "render-audit")]
fn audit_authored_visibility(chart: &Chart, line_id: usize, note_id: usize, start: f64, hit: f64, playback_rate: f64) -> serde_json::Value {
    let Some(steps) = validation_steps(start, hit, playback_rate) else {
        return serde_json::json!({"validation_budget":false});
    };
    let mut hidden_reasons = std::collections::BTreeMap::<&'static str, usize>::new();
    let mut hidden_at_target = false;
    let mut hidden_spans = Vec::<[f64; 2]>::new();
    let mut hidden_span_start = None;
    let mut previous_time = start;
    for i in 0..=steps {
        let time = if i == steps {
            hit - (hit - start).min(1e-7)
        } else {
            start + (hit - start) * i as f64 / steps as f64
        };
        if let Some(reason) = authored_alpha_gate_at(chart, line_id, note_id, time) {
            *hidden_reasons.entry(reason).or_default() += 1;
            hidden_at_target |= i == 0;
            hidden_span_start.get_or_insert(time);
        } else if let Some(span_start) = hidden_span_start.take() {
            hidden_spans.push([span_start, previous_time]);
        }
        previous_time = time;
    }
    if let Some(span_start) = hidden_span_start {
        hidden_spans.push([span_start, previous_time]);
    }
    serde_json::json!({
        "validation_budget":true,
        "samples":steps+1,
        "hidden_samples":hidden_reasons.values().sum::<usize>(),
        "hidden_at_target":hidden_at_target,
        "hidden_spans":hidden_spans,
        "hidden_reasons":hidden_reasons,
    })
}

fn note_plan(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, preview_chart_seconds: f64) -> PreviewPlan {
    let note = &chart.lines[line_id].notes[note_id];
    choose_plan(note.fake, res.note_flow_speed, note.time - preview_chart_seconds, note.time, res.config.speed as f64, |time, flow| {
        note_geometry_at(chart, line_id, note_id, res, time, flow).map(|geometry| PreviewSample {
            anchor: geometry.anchor,
            visible: geometry_visible(geometry.quads),
        })
    })
}

impl Chart {
    /// Host diagnostics expose the solver's bracket and decision separately from
    /// the production renderer oracle. This never changes cached flows.
    #[cfg(feature = "render-audit")]
    pub fn audit_preview_solver(&self, res: &Resource, preview_chart_seconds: f64) -> Vec<serde_json::Value> {
        self.lines.iter().enumerate().flat_map(|(line_id, line)| {
            line.notes.iter().enumerate().map(move |(note_id, note)| {
                let target = note.time - preview_chart_seconds;
                let minimum = note_quad_at(self, line_id, note_id, res, target, NOTE_FLOW_SPEED_MIN);
                let maximum = note_quad_at(self, line_id, note_id, res, target, NOTE_FLOW_SPEED_MAX);
                let min_visible = minimum.is_some_and(geometry_visible);
                let max_visible = maximum.is_some_and(geometry_visible);
                let boundary_flow = solve_flow(
                    |flow| note_quad_at(self, line_id, note_id, res, target, flow).is_some_and(geometry_visible),
                    NOTE_FLOW_SPEED_MIN,
                    NOTE_FLOW_SPEED_MAX,
                );
                let plan = note_plan(self, line_id, note_id, res, preview_chart_seconds);
                let minimum_candidate = audit_constant_candidate(
                    self,
                    line_id,
                    note_id,
                    res,
                    target,
                    note.time,
                    res.config.speed as f64,
                    NOTE_FLOW_SPEED_MIN,
                );
                let boundary_candidate = boundary_flow.map(|flow| audit_constant_candidate(
                    self,
                    line_id,
                    note_id,
                    res,
                    target,
                    note.time,
                    res.config.speed as f64,
                    flow,
                ));
                let original_candidate = audit_constant_candidate(
                    self,
                    line_id,
                    note_id,
                    res,
                    target,
                    note.time,
                    res.config.speed as f64,
                    res.note_flow_speed,
                );
                let authored_visibility = audit_authored_visibility(
                    self,
                    line_id,
                    note_id,
                    target,
                    note.time,
                    res.config.speed as f64,
                );
                let no_bracket_probe = boundary_flow.is_none().then(|| {
                    let mut visible_count = 0_usize;
                    let mut first_visible = None;
                    let mut last_visible = None;
                    for i in 0..=64 {
                        let flow = NOTE_FLOW_SPEED_MIN
                            + (NOTE_FLOW_SPEED_MAX - NOTE_FLOW_SPEED_MIN) * i as f32 / 64.;
                        if note_quad_at(self, line_id, note_id, res, target, flow).is_some_and(geometry_visible) {
                            visible_count += 1;
                            first_visible.get_or_insert(flow);
                            last_visible = Some(flow);
                        }
                    }
                    serde_json::json!({"samples":65,"visible_count":visible_count,"first_visible":first_visible,"last_visible":last_visible})
                });
                serde_json::json!({"line":line_id,"note":note_id,"time":note.time,"kind":format!("{:?}", note.kind),"fake":note.fake,"target_chart_time":target,"flow":plan.flow(),"plan":plan.name(),"reason":plan.reason(),"minimum_visible":min_visible,"maximum_visible":max_visible,"minimum_has_geometry":minimum.is_some(),"maximum_has_geometry":maximum.is_some(),"target_boundary_flow":boundary_flow,"minimum_candidate_audit":minimum_candidate,"boundary_candidate_audit":boundary_candidate,"original_candidate_audit":original_candidate,"authored_visibility_audit":authored_visibility,"no_bracket_probe":no_bracket_probe,"validation_hz":VALIDATION_HZ,"validation_max_steps":MAX_VALIDATION_STEPS,"constant_motion_absolute_limit_ndc_per_real_second":CONSTANT_MOTION_ABSOLUTE_LIMIT_NDC_PER_REAL_SECOND,"constant_motion_original_tolerance_ndc_per_real_second":CONSTANT_MOTION_ORIGINAL_TOLERANCE_NDC_PER_REAL_SECOND,"constant_motion_metric":"semantic_head_anchor_ndc_per_real_second"})
            })
        }).collect()
    }

    pub(crate) fn clear_fixed_preview(&mut self) {
        for line in &mut self.lines {
            line.cache.set_fixed_preview_plans(Vec::new());
        }
    }

    pub(crate) fn rebuild_fixed_preview(&mut self, res: &Resource, preview_chart_seconds: f64) -> RebuildStats {
        let mut stats = RebuildStats::default();
        let plans: Vec<Vec<PreviewPlan>> = self
            .lines
            .iter()
            .enumerate()
            .map(|(line_id, line)| {
                line.notes
                    .iter()
                    .enumerate()
                    .map(|(note_id, note)| {
                        let plan = note_plan(self, line_id, note_id, res, preview_chart_seconds);
                        match plan {
                            PreviewPlan::Constant { .. } => stats.solved += 1,
                            PreviewPlan::GateOnly { .. } => stats.gate_only += 1,
                            PreviewPlan::Original { .. } if !note.fake => stats.fallback += 1,
                            _ => {}
                        }
                        plan
                    })
                    .collect()
            })
            .collect();
        for (line, plans) in self.lines.iter_mut().zip(plans) {
            line.cache.set_fixed_preview_plans(plans);
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_visibility_covers_containment_and_rejects_corner_only_aabb_overlap() {
        assert!(quad_visible([Point::new(-2., -0.1), Point::new(2., -0.1), Point::new(2., 0.1), Point::new(-2., 0.1),]));
        assert!(!quad_visible([Point::new(1.1, 2.), Point::new(2., 1.1), Point::new(2.2, 1.3), Point::new(1.3, 2.2),]));
        assert!(!segments_intersect(Point::new(2., 1.), Point::new(3., 1.), Point::new(-1., 1.), Point::new(1., 1.)));
    }

    #[test]
    fn bounded_solver_requires_a_bracket_and_converges() {
        let solved = solve_flow(|flow| flow <= 3.25, 0.1, 20.).unwrap();
        assert!((solved - 3.25).abs() < 0.001);
        assert_eq!(solve_flow(|_| true, 0.1, 20.), None);
        assert_eq!(solve_flow(|_| false, 0.1, 20.), None);
    }
}
