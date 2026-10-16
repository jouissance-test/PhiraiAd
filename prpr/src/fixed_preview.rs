//! Fixed Preview Time's bounded solver and application preference bridge.

use crate::{
    config::{NOTE_FLOW_SPEED_MAX, NOTE_FLOW_SPEED_MIN},
    core::{Chart, Matrix, Note, NoteKind, Object, Point, Resource, Vector},
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RebuildStats {
    pub solved: usize,
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
    Rotation2::new(line_rotation_at(chart, id, time).to_radians())
        .to_homogeneous()
        .append_translation(&line_position_at(chart, id, time, aspect_ratio))
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

fn note_quad_at(chart: &Chart, line_id: usize, note_id: usize, res: &Resource, time: f64, flow: f32) -> Option<[Point; 4]> {
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
    let mut translation = object_translation_at(&note.object, time, res.aspect_ratio);
    let mut note_scale = object_scale_at(&note.object, time);
    note_scale.x *= ctrl_size;
    if res.info.note_uniform_scale {
        note_scale.y *= ctrl_size;
    } else {
        note_scale.y = 1.;
    }
    if translation.iter().chain(note_scale.iter()).any(|value| !value.is_finite()) {
        return None;
    }

    let multiple_scale = if res.config.double_hint && note.multiple_hint {
        res.res_pack.note_style_mh.click.width() / res.res_pack.note_style.click.width()
    } else {
        1.
    };
    let half_width = res.note_width * multiple_scale;
    let (note_transform, local) = match note.kind {
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
            let low = head.min(tail).min(head_y).min(tail_y);
            let high = head.max(tail).max(head_y + head_half * 2.).max(tail_y + tail_half * 2.);
            let transform = Rotation2::new(sample(&note.object.rotation, time, 0.).to_radians())
                .to_homogeneous()
                .append_nonuniform_scaling(&note_scale)
                .append_translation(&translation);
            (transform, local_rect(-half_width, low, half_width, high))
        }
        _ => {
            let scaled_base = base * flow;
            let incline_sin = sample(&line.incline, time, 0.).to_radians().sin();
            let incline = 1. - incline_sin * (scaled_base * res.aspect_ratio + y_offset) * RPE_HEIGHT / 2. / 360.;
            translation.x *= incline * ctrl_pos;
            translation.y += scaled_base;
            let texture = texture_for(note, res);
            let half_height = texture.height() / texture.width() * half_width;
            let transform = Rotation2::new(sample(&note.object.rotation, time, 0.).to_radians())
                .to_homogeneous()
                .append_nonuniform_scaling(&note_scale)
                .append_translation(&translation);
            (transform, local_rect(-half_width, -half_height, half_width, half_height))
        }
    };

    let line_scale = object_scale_at(&line.object, time);
    if line_scale.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let (flip_x, flip_y) = crate::chart_play::reflection_axes(res.config.flip_x(), false);
    let mut transform = Matrix::identity().append_nonuniform_scaling(&Vector::new(flip_x, -flip_y))
        * line_transform_at(chart, line_id, time, res.aspect_ratio)
        * Matrix::identity().append_nonuniform_scaling(&line_scale);
    if !note.above {
        transform *= Matrix::identity().append_nonuniform_scaling(&Vector::new(1., -1.));
    }
    transform *= note_transform;
    let quad = local.map(|point| preview_ndc(res, transform.transform_point(&point)));
    quad.iter().all(|point| point.x.is_finite() && point.y.is_finite()).then_some(quad)
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
                let min_visible = minimum.is_some_and(quad_visible);
                let max_visible = maximum.is_some_and(quad_visible);
                let solved = if !note.fake && target >= 0. && target.is_finite() {
                    solve_flow(|flow| note_quad_at(self, line_id, note_id, res, target, flow).is_some_and(quad_visible), NOTE_FLOW_SPEED_MIN, NOTE_FLOW_SPEED_MAX)
                } else { None };
                let reason = if note.fake { "fake_note_not_applicable" }
                    else if !target.is_finite() || target < 0. { "target_before_chart_or_nonfinite" }
                    else if minimum.is_none() { "no_geometry_at_minimum" }
                    else if !min_visible { "minimum_not_visible" }
                    else if max_visible { "maximum_still_visible" }
                    else if solved.is_some() { "solved" } else { "no_valid_bracket" };
                serde_json::json!({"line":line_id,"note":note_id,"time":note.time,"kind":format!("{:?}", note.kind),"fake":note.fake,"target_chart_time":target,"flow":solved.unwrap_or(res.note_flow_speed),"reason":reason,"minimum_visible":min_visible,"maximum_visible":max_visible,"minimum_has_geometry":minimum.is_some(),"maximum_has_geometry":maximum.is_some()})
            })
        }).collect()
    }

    pub(crate) fn clear_fixed_preview(&mut self) {
        for line in &mut self.lines {
            line.cache.set_fixed_preview_flows(Vec::new());
        }
    }

    pub(crate) fn rebuild_fixed_preview(&mut self, res: &Resource, preview_chart_seconds: f64) -> RebuildStats {
        let mut stats = RebuildStats::default();
        let flows: Vec<Vec<f32>> = self
            .lines
            .iter()
            .enumerate()
            .map(|(line_id, line)| {
                line.notes
                    .iter()
                    .enumerate()
                    .map(|(note_id, note)| {
                        if !crate::fixed_preview_policy::applies_to_note(note.fake) {
                            return res.note_flow_speed;
                        }
                        let target_time = note.time - preview_chart_seconds;
                        let solved = (target_time >= 0. && target_time.is_finite()).then(|| {
                            solve_flow(
                                |flow| note_quad_at(self, line_id, note_id, res, target_time, flow).is_some_and(quad_visible),
                                NOTE_FLOW_SPEED_MIN,
                                NOTE_FLOW_SPEED_MAX,
                            )
                        });
                        if let Some(flow) = solved.flatten() {
                            stats.solved += 1;
                            flow
                        } else {
                            stats.fallback += 1;
                            res.note_flow_speed
                        }
                    })
                    .collect()
            })
            .collect();
        for (line, flows) in self.lines.iter_mut().zip(flows) {
            line.cache.set_fixed_preview_flows(flows);
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
