//! Execute the production geometry sampler and bounded solver without a GPU.
//!
//! Adapters supply static dimensions and scene state; matrix math, animation
//! sampling, viewport intersection and flow solving come from production code.
//! These tests isolate carrier opacity. They do not emulate line animation,
//! rendering, negative-alpha visibility extensions or texture loading.
#![allow(dead_code)]

use nalgebra::Rotation2;
use std::cell::RefCell;

mod core {
    use super::*;
    pub use phira_chart_play_tests::anim::{AnimFloat, AnimVector};
    pub type Matrix = nalgebra::Matrix3<f32>;
    pub type Point = nalgebra::Point2<f32>;
    pub type Vector = nalgebra::Vector2<f32>;

    #[derive(Default)]
    pub struct Object {
        pub translation: AnimVector,
        pub scale: AnimVector,
        pub rotation: AnimFloat,
        pub alpha: AnimFloat,
    }
    #[derive(Clone, Copy)]
    pub enum NoteKind {
        Click,
        Flick,
        Drag,
        Hold { end_time: f64, end_height: f64 },
    }
    pub struct Note {
        pub object: Object,
        pub kind: NoteKind,
        pub multiple_hint: bool,
        pub above: bool,
        pub fake: bool,
        pub time: f64,
        pub height: f64,
        pub speed: f64,
    }
    #[derive(Default)]
    pub struct CtrlObject {
        pub y: AnimFloat,
        pub size: AnimFloat,
        pub pos: AnimFloat,
        pub alpha: AnimFloat,
    }
    pub struct Line {
        pub object: Object,
        pub parent: Option<usize>,
        pub rot_with_parent: bool,
        pub height: AnimFloat,
        pub incline: AnimFloat,
        pub ctrl_obj: RefCell<CtrlObject>,
        pub notes: Vec<Note>,
    }
    pub struct Chart {
        pub lines: Vec<Line>,
    }
    #[derive(Default)]
    pub struct Config {
        pub double_hint: bool,
        pub shorten_holds: bool,
        pub speed: f32,
    }
    impl Config {
        pub fn flip_x(&self) -> bool {
            false
        }
    }
    pub struct Camera {
        pub viewport: Option<(i32, i32, i32, i32)>,
    }
    pub struct PracticeView;
    impl PracticeView {
        pub fn center(&self, _width: f32) -> (f32, f32) {
            (0., 0.)
        }
        pub fn scale(&self) -> f32 {
            1.
        }
    }
    pub struct Info {
        pub note_uniform_scale: bool,
    }
    pub struct Resource {
        pub config: Config,
        pub camera: Camera,
        pub practice_view: PracticeView,
        pub last_vp: (i32, i32, i32, i32),
        pub aspect_ratio: f32,
        pub note_width: f32,
        pub info: Info,
        pub res_pack: ResourcePack,
    }
    pub struct ResourcePack {
        pub note_style: NoteStyle,
        pub note_style_mh: NoteStyle,
        pub info: PackInfo,
    }
    pub struct PackInfo {
        pub hold_compact: bool,
    }
    pub struct NoteStyle {
        pub click: Box<Texture2D>,
        pub flick: Box<Texture2D>,
        pub drag: Box<Texture2D>,
        pub hold: Box<Texture2D>,
    }
    pub struct Rect {
        pub w: f32,
        pub h: f32,
    }
    impl NoteStyle {
        pub fn hold_ratio(&self) -> f32 {
            1.
        }
        pub fn hold_head_rect(&self) -> Rect {
            Rect { w: 100., h: 10. }
        }
        pub fn hold_tail_rect(&self) -> Rect {
            self.hold_head_rect()
        }
    }
}

#[derive(Clone, Copy)]
struct Texture2D;
impl Texture2D {
    fn width(&self) -> f32 {
        100.
    }
    fn height(&self) -> f32 {
        10.
    }
}
mod parse {
    pub const RPE_HEIGHT: f32 = 900.;
}
mod chart_play {
    pub use phira_chart_play_tests::chart_play::{hold_render_end_height, hold_visual_end, reflection_axes};
}
mod judge {
    // Unused by these Click-only fixtures; the sampler also contains Hold code.
    pub fn hold_tail_window(_config: &super::core::Config) -> f64 {
        0.1
    }
}
use core::{Chart, Matrix, Note, NoteKind, Object, Point, Resource, Vector};
use parse::RPE_HEIGHT;

include!(concat!(env!("OUT_DIR"), "/fixed_preview_geometry.rs"));

fn resource() -> Resource {
    fn style() -> core::NoteStyle {
        core::NoteStyle {
            click: Box::new(Texture2D),
            flick: Box::new(Texture2D),
            drag: Box::new(Texture2D),
            hold: Box::new(Texture2D),
        }
    }
    Resource {
        config: core::Config {
            speed: 1.,
            ..Default::default()
        },
        camera: core::Camera { viewport: None },
        practice_view: core::PracticeView,
        last_vp: (0, 0, 1920, 1080),
        aspect_ratio: 16. / 9.,
        note_width: 0.06,
        info: core::Info { note_uniform_scale: false },
        res_pack: core::ResourcePack {
            note_style: style(),
            note_style_mh: style(),
            info: core::PackInfo { hold_compact: false },
        },
    }
}

fn carrier_chart(line_alpha: f32) -> Chart {
    Chart {
        lines: vec![core::Line {
            object: Object {
                alpha: core::AnimFloat::fixed(line_alpha),
                ..Default::default()
            },
            parent: None,
            rot_with_parent: false,
            height: core::AnimFloat::fixed(0.),
            incline: core::AnimFloat::fixed(0.),
            ctrl_obj: RefCell::new(core::CtrlObject::default()),
            notes: vec![Note {
                object: Object::default(),
                kind: NoteKind::Click,
                multiple_hint: false,
                above: true,
                fake: false,
                time: 10.,
                height: 4.,
                speed: 1.,
            }],
        }],
    }
}

fn solved(chart: &Chart, res: &Resource, time: f64) -> Option<f32> {
    solve_flow(|flow| note_quad_at(chart, 0, 0, res, time, flow).is_some_and(quad_visible), 0.1, 20.)
}

#[test]
fn line_opacity_does_not_change_note_geometry_or_flow() {
    let res = resource();
    let chart = carrier_chart(1.);
    let quad = note_quad_at(&chart, 0, 0, &res, 9.2, 0.2).unwrap();
    let flow = solved(&chart, &res, 9.2).unwrap();
    for alpha in [0., -1. / 255.] {
        let chart = carrier_chart(alpha);
        assert_eq!(note_quad_at(&chart, 0, 0, &res, 9.2, 0.2), Some(quad), "line alpha={alpha}");
        assert_eq!(solved(&chart, &res, 9.2), Some(flow), "line alpha={alpha}");
    }
}

#[test]
fn transparent_note_or_control_still_has_no_flow_solution() {
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].notes[0].object.alpha = core::AnimFloat::fixed(0.);
    assert_eq!(solved(&chart, &res, 9.2), None);
    chart.lines[0].notes[0].object.alpha = core::AnimFloat::fixed(1.);
    chart.lines[0].ctrl_obj.borrow_mut().alpha = core::AnimFloat::fixed(0.);
    assert_eq!(solved(&chart, &res, 9.2), None);
}

#[test]
fn waking_shadows_transparent_carrier_does_not_force_original_speed() {
    // Real chart line 1, original JSON note 4: beat161+1/8, Drag.
    // Alpha=255, visibleTime=999999,
    // speed=1, above=1, positionX=-306.818182, yOffset=0. The line's
    // target at preview0.8 lies during its entrance near beat158.592.
    // Freeze the moving scene to isolate opacity, without claiming to
    // reproduce the chart's full time-varying height/position curves.
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].object.translation.1 = core::AnimFloat::fixed(-0.9);
    chart.lines[0].notes[0].kind = NoteKind::Drag;
    chart.lines[0].notes[0].time = 50.88157894736842;
    chart.lines[0].notes[0].object.translation.0 = core::AnimFloat::fixed(-306.818182 / 675.);
    let time = chart.lines[0].notes[0].time - 0.8;
    let expected = solved(&chart, &res, time).unwrap();
    assert!(expected < 1.);
    chart.lines[0].object.alpha = core::AnimFloat::fixed(0.);
    assert_eq!(solved(&chart, &res, time), Some(expected));
}
