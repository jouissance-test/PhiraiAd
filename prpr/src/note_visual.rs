//! Small shared geometry seam for production drawing and fixed-preview sampling.
//! Inputs are sampled by their callers; this module has no animation cursors,
//! allocations, visibility search, chart time or judgement state.
use crate::{
    core::{Matrix, Vector},
    parse::RPE_HEIGHT,
};
use nalgebra::Rotation2;

pub(crate) struct NoteTransform {
    pub translation: Vector,
    pub scale: Vector,
    pub rotation: f32,
    pub y_offset: f32,
    pub ctrl_size: f32,
    pub ctrl_pos: f32,
    pub uniform_scale: bool,
    pub hold: bool,
    pub incline_sin: f32,
    pub aspect_ratio: f32,
}

impl NoteTransform {
    #[inline]
    pub fn matrix(mut self, base: f32, flow: f32) -> Matrix {
        let base = base * flow;
        let incline = 1. - self.incline_sin * (base * self.aspect_ratio + self.y_offset) * RPE_HEIGHT / 2. / 360.;
        if !self.hold {
            self.translation.x *= incline * self.ctrl_pos;
        }
        self.translation.y += base;
        self.scale.x *= self.ctrl_size;
        self.scale.y = if self.uniform_scale { self.scale.y * self.ctrl_size } else { 1. };
        Rotation2::new(self.rotation.to_radians())
            .to_homogeneous()
            .append_nonuniform_scaling(&self.scale)
            .append_translation(&self.translation)
    }
}

/// Carrier scale affects only the carrier drawing, never its attached Notes.
#[inline]
pub(crate) fn line_transform(rotation: f32, position: Vector) -> Matrix {
    Rotation2::new(rotation.to_radians()).to_homogeneous().append_translation(&position)
}

#[inline]
pub(crate) fn covered_before_hit(draw_below: bool, now: f64, hit: f64, cover_base: f64) -> bool {
    !draw_below && now < hit && cover_base <= -0.001
}
