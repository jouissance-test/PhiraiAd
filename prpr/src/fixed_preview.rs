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
// between samples. Two candidates cost at most 3074 geometry evaluations plus
// the 18-call target bracket search; unsupported durations fall back unchanged.
const VALIDATION_HZ: f64 = 240.;
const MAX_VALIDATION_STEPS: usize = 1536;
// Equivalent displacement in one 240Hz real-time frame: 0.1 NDC is 5% of
// the viewport width, or 24 NDC/real second. Only newly solved Constant motion
// is constrained; GateOnly preserves the chart's authored trajectory.
const CONSTANT_MOTION_STEP_LIMIT_NDC: f64 = 0.1;

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

fn choose_plan(
    fake: bool,
    original_flow: f32,
    start: f64,
    hit: f64,
    playback_rate: f64,
    mut sample: impl FnMut(f64, f32) -> Option<Point>,
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
    if visible_interval(start, hit, steps, |time| sample(time, original_flow).is_some()) {
        return PreviewPlan::GateOnly { original_flow, start };
    }
    let Some(flow) = solve_flow(|flow| sample(start, flow).is_some(), NOTE_FLOW_SPEED_MIN, NOTE_FLOW_SPEED_MAX) else {
        return original(FallbackReason::NoBracket);
    };
    let mut previous: Option<(f64, Point)> = None;
    let mut motion_spike = false;
    let accepted = visible_interval(start, hit, steps, |time| {
        let Some(anchor) = sample(time, flow) else { return false };
        if let Some((previous_time, previous_anchor)) = previous {
            let real_dt = (time - previous_time) / playback_rate;
            let equivalent_step = (anchor - previous_anchor).norm() as f64 / real_dt / VALIDATION_HZ;
            if !equivalent_step.is_finite() || equivalent_step > CONSTANT_MOTION_STEP_LIMIT_NDC {
                motion_spike = true;
                return false;
            }
        }
        previous = Some((time, anchor));
        true
    });
    if accepted {
        PreviewPlan::Constant { flow, start }
    } else {
        original(if motion_spike {
            FallbackReason::MotionSpike
        } else {
            FallbackReason::VisibilityGap
        })
    }
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

fn note_geometry_at(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, time: f64, flow: f32) -> Option<NoteGeometry> {
    let line = &chart.lines[line_id];
    let note = &line.notes[note_id];
    if !time.is_finite() || !flow.is_finite() || flow <= 0. || res.aspect_ratio.abs() <= f32::EPSILON {
        return None;
    }
    // Carrier opacity does not affect note geometry: transparent lines can still
    // carry visible notes. The renderer owns negative-alpha visibility extensions.
    if sample(&note.object.alpha, time, 1.) <= 0. {
        return None;
    }

    let line_height = sample(&line.height, time, 0.) as f64;
    let y_offset = sample(&note.object.translation.1, time, 0.);
    if note.speed.abs() <= f64::EPSILON {
        return None;
    }
    let ctrl_height = (note.height - line_height + y_offset as f64 / note.speed) * RPE_HEIGHT as f64 / 2.;
    let ctrl = line.ctrl_obj.borrow();
    let ctrl_y = sample(&ctrl.y, ctrl_height, 1.);
    let ctrl_size = sample(&ctrl.size, ctrl_height, 1.);
    let ctrl_pos = sample(&ctrl.pos, ctrl_height, 1.);
    let ctrl_alpha = sample(&ctrl.alpha, ctrl_height, 1.);
    if [ctrl_y, ctrl_size, ctrl_pos, ctrl_alpha].iter().any(|value| !value.is_finite()) || ctrl_alpha <= 0. {
        return None;
    }

    let speed = note.speed * ctrl_y as f64;
    let base = ((note.height - line_height) / res.aspect_ratio as f64 * speed) as f32;
    let translation = object_translation_at(&note.object, time, res.aspect_ratio);
    let note_scale = object_scale_at(&note.object, time);
    if translation.iter().chain(note_scale.iter()).any(|value| !value.is_finite()) {
        return None;
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
    let (note_transform, local, cover_base, anchor_local) = match note.kind {
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
            let cover_base = if chart.settings.hold_partial_cover {
                (end_height - line_height) / res.aspect_ratio as f64 * speed
            } else {
                base as f64
            };
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
                cover_base,
                Point::new(0., head),
            )
        }
        _ => {
            let texture = texture_for(note, res);
            let half_height = texture.height() / texture.width() * half_width;
            (geometry.matrix(base, flow), [local_rect(-half_width, -half_height, half_width, half_height); 3], base as f64, Point::origin())
        }
    };
    let line_alpha = sample(&line.object.alpha, time, 1.);
    let mut draw_below = line.show_below;
    if line_alpha < 0. {
        if !chart.settings.pe_alpha_extension {
            return None;
        }
        match (-line_alpha).floor() as u32 {
            1 => return None,
            2 => draw_below = false,
            // Beat-based appearance and unsupported negative-alpha modes are
            // deliberately unplanned until their temporal gates are shared.
            100..=1999 => return None,
            _ => {}
        }
    }
    if crate::note_visual::covered_before_hit(draw_below, time, note.time, cover_base) {
        return None;
    }
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
    quads
        .iter()
        .flatten()
        .chain(std::iter::once(&anchor))
        .all(|point| point.x.is_finite() && point.y.is_finite())
        .then_some(NoteGeometry { quads, anchor })
}

fn note_quad_at(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, time: f64, flow: f32) -> Option<[[Point; 4]; 3]> {
    note_geometry_at(chart, line_id, note_id, res, time, flow).map(|geometry| geometry.quads)
}

fn geometry_visible(quads: [[Point; 4]; 3]) -> bool {
    quads.into_iter().any(quad_visible)
}

fn note_plan(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, preview_chart_seconds: f64) -> PreviewPlan {
    let note = &chart.lines[line_id].notes[note_id];
    choose_plan(note.fake, res.note_flow_speed, note.time - preview_chart_seconds, note.time, res.config.speed as f64, |time, flow| {
        note_geometry_at(chart, line_id, note_id, res, time, flow)
            .filter(|geometry| geometry_visible(geometry.quads))
            .map(|geometry| geometry.anchor)
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
                let plan = note_plan(self, line_id, note_id, res, preview_chart_seconds);
                serde_json::json!({"line":line_id,"note":note_id,"time":note.time,"kind":format!("{:?}", note.kind),"fake":note.fake,"target_chart_time":target,"flow":plan.flow(),"plan":plan.name(),"reason":plan.reason(),"minimum_visible":min_visible,"maximum_visible":max_visible,"minimum_has_geometry":minimum.is_some(),"maximum_has_geometry":maximum.is_some(),"validation_hz":VALIDATION_HZ,"validation_max_steps":MAX_VALIDATION_STEPS,"constant_motion_step_limit_ndc":CONSTANT_MOTION_STEP_LIMIT_NDC,"constant_motion_speed_limit_ndc_per_real_second":CONSTANT_MOTION_STEP_LIMIT_NDC*VALIDATION_HZ,"constant_motion_metric":"semantic_head_anchor_ndc_per_real_second"})
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
