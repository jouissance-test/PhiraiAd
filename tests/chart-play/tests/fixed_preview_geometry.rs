//! Execute the production geometry sampler and bounded solver without a GPU.
//!
//! Adapters supply static dimensions and scene state; matrix math, animation
//! sampling, viewport intersection and flow solving come from production code.
//! Renderer-owned alpha/appearance gates stay outside the geometry solver.
//! These tests do not emulate final texture alpha or GPU rendering.
#![allow(dead_code)]

use nalgebra::Rotation2;
use phira_chart_play_tests::{
    config::{NOTE_FLOW_SPEED_MAX, NOTE_FLOW_SPEED_MIN},
    fixed_preview_policy::{FallbackReason, PreviewPlan},
};
use std::cell::RefCell;
#[path = "../../../prpr/src/note_visual.rs"]
mod note_visual;

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
        pub show_below: bool,
    }
    pub struct Chart {
        pub lines: Vec<Line>,
        pub settings: ChartSettings,
    }
    pub struct ChartSettings {
        pub hold_partial_cover: bool,
        pub pe_alpha_extension: bool,
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
        pub note_flow_speed: f32,
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
        note_flow_speed: 1.,
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
        settings: core::ChartSettings {
            hold_partial_cover: false,
            pe_alpha_extension: true,
        },
        lines: vec![core::Line {
            show_below: true,
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
    solve_flow(|flow| note_quad_at(chart, 0, 0, res, time, flow).is_some_and(geometry_visible), 0.1, 20.)
}

#[test]
fn reversing_scroll_must_not_accept_a_single_target_boundary_contact() {
    // Minimal version of the Waking Shadows 62.246s failure: the integrated
    // height moves away from the hit line after the target, then reverses.
    // A boundary-only flow near 6.9 disappears; authored flow 1 stays visible.
    use phira_chart_play_tests::anim::Keyframe;
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].notes[0].height = 0.;
    chart.lines[0].height = core::AnimFloat::new(vec![Keyframe::new(9.55, -0.145, 0), Keyframe::new(9.65, -0.5, 0), Keyframe::new(10., 0., 0)]);
    let old_boundary_flow = solved(&chart, &res, 9.55).unwrap();
    assert!(old_boundary_flow > 6.);
    assert!(!note_quad_at(&chart, 0, 0, &res, 9.65, old_boundary_flow).is_some_and(geometry_visible));
    let plan = note_plan(&chart, 0, 0, &res, 0.45);
    assert!(matches!(plan, PreviewPlan::GateOnly { original_flow: 1., .. }));
    let accepted_flow = plan.flow();
    for i in 0..=108 {
        let time = 9.55 + 0.45 * i as f64 / 108.;
        assert!(
            note_quad_at(&chart, 0, 0, &res, time, accepted_flow).is_some_and(geometry_visible),
            "accepted flow {accepted_flow} leaves the view at {time}"
        );
    }
}

#[test]
fn constant_candidate_may_inherit_but_not_amplify_authored_line_motion() {
    use phira_chart_play_tests::anim::Keyframe;
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].height = core::AnimFloat::new(vec![Keyframe::new(9.55, 0., 0), Keyframe::new(10., 4., 0)]);
    chart.lines[0].object.translation.0 = core::AnimFloat::new(vec![
        Keyframe::new(9.55, -0.4, 0),
        Keyframe::new(9.75, -0.4, 0),
        Keyframe::new(9.75001, 0.4, 0),
        Keyframe::new(10., 0.4, 0),
    ]);
    let candidate = solved(&chart, &res, 9.55).unwrap();
    assert!(candidate < 1.);
    for i in 0..=108 {
        let time = 9.55 + 0.45 * i as f64 / 108.;
        assert!(note_quad_at(&chart, 0, 0, &res, time, candidate).is_some_and(geometry_visible));
    }
    let plan = note_plan(&chart, 0, 0, &res, 0.45);
    assert!(matches!(plan, PreviewPlan::Constant { .. }), "the candidate does not exceed Original's inherited jump: {plan:?}");
}

#[test]
fn gate_only_preserves_authored_motion_spikes() {
    let plan = choose_plan(false, 1., 9.55, 10., 1., |time, _| {
        Some(PreviewSample {
            anchor: Point::new(if time < 9.75 { -0.4 } else { 0.4 }, 0.),
            visible: true,
        })
    });
    assert!(matches!(plan, PreviewPlan::GateOnly { original_flow: 1., .. }));
}

#[test]
fn constant_motion_guard_normalizes_chart_steps_to_real_time() {
    // Nonintegral duration*240 exercises ceil-spaced steps and hit-left-limit.
    // Both playback rates must apply the same 24-NDC/real-second threshold.
    for rate in [0.5, 1., 2.] {
        for (real_velocity, rejected) in [(23.9, false), (24.1, true)] {
            let start = 9.54321;
            let plan = choose_plan(false, 1., start, 10., rate, |time, flow| {
                Some(PreviewSample {
                    anchor: Point::new(if flow <= 0.5 { ((time - start) * real_velocity / rate) as f32 } else { 0. }, 0.),
                    visible: flow <= 0.5,
                })
            });
            assert_eq!(plan.reason() == "motion_spike", rejected, "rate={rate}, velocity={real_velocity}, {plan:?}");
            if !rejected {
                assert!(matches!(plan, PreviewPlan::Constant { .. }));
            }
        }
    }
}

#[test]
fn hold_motion_anchor_is_semantic_head_and_ignores_tail_and_skin_offsets() {
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].notes[0].height = 0.4;
    chart.lines[0].notes[0].kind = NoteKind::Hold {
        end_time: 11.,
        end_height: 5.,
    };
    let first = note_geometry_at(&chart, 0, 0, &res, 9.55, 0.5).unwrap();
    // Reflection maps the semantic pre-hit head to -height*flow in NDC.
    assert!((first.anchor.y + 0.2).abs() < 1e-6);
    chart.lines[0].notes[0].kind = NoteKind::Hold {
        end_time: 11.,
        end_height: 0.5,
    };
    let mut compact = resource();
    compact.res_pack.info.hold_compact = true;
    let second = note_geometry_at(&chart, 0, 0, &compact, 9.55, 0.5).unwrap();
    assert_eq!(first.anchor, second.anchor);
    assert_ne!(first.quads, second.quads);
}

#[test]
fn interval_failure_restores_original_flow_and_has_no_time_gate() {
    // Both flows leave the viewport during the interval. The target still has
    // a valid bisection bracket, so rejecting only failed brackets is unsafe.
    let plan = choose_plan(false, 1., 9.55, 10., 1., |time, flow| {
        let distance = if time > 9.56 && time < 9.9 { 20_000. } else { 0.145 };
        Some(PreviewSample {
            anchor: Point::origin(),
            visible: distance * flow <= 1.,
        })
    });
    assert_eq!(
        plan,
        PreviewPlan::Original {
            original_flow: 1.,
            reason: FallbackReason::VisibilityGap
        }
    );
    assert!(!plan.hides(false, 0.));
    assert_eq!(plan.flow(), 1.);
}

#[test]
fn smaller_constant_candidate_can_rescue_a_failed_target_boundary_candidate() {
    // The largest flow visible at the target crosses an authored moving
    // viewport boundary later in the approach. A smaller constant flow stays
    // visible for the complete interval, so falling straight back to Original
    // needlessly gives up Fixed Preview coverage.
    let plan = choose_plan(false, 1., 9.55, 10., 1., |time, flow| {
        let limit = if (9.7..9.8).contains(&time) { 0.5 } else { 1. };
        Some(PreviewSample {
            anchor: Point::origin(),
            visible: flow <= limit,
        })
    });
    assert!(
        matches!(plan, PreviewPlan::Constant { flow, start: 9.55 } if flow <= 0.5 && flow >= NOTE_FLOW_SPEED_MIN),
        "a safe smaller constant candidate should be selected: {plan:?}"
    );
}

#[test]
fn monotone_approach_can_use_constant_flow_when_original_is_outside_at_target() {
    let plan = choose_plan(false, 1., 9., 10., 1., |time, flow| {
        Some(PreviewSample {
            anchor: Point::origin(),
            visible: (10. - time) * 2. * flow as f64 <= 1.,
        })
    });
    assert!(matches!(plan, PreviewPlan::Constant { flow, start: 9. } if (flow - 0.5).abs() < 0.001));
    assert!(plan.hides(false, 8.99));
    assert!(!plan.hides(false, 9.));
}

#[test]
fn validation_rejects_late_gap_and_caps_work_independent_of_duration() {
    let mut calls = 0;
    let plan = choose_plan(false, 1., 0., 100., 1., |_, _| {
        calls += 1;
        Some(PreviewSample {
            anchor: Point::origin(),
            visible: true,
        })
    });
    assert_eq!(plan.reason(), "validation_budget_exceeded");
    assert_eq!(calls, 0);
    for gap in [9.5501..9.57, 9.7..9.73, 9.99..10.] {
        let plan = choose_plan(false, 1., 9.55, 10., 1., |time, flow| {
            Some(PreviewSample {
                anchor: Point::origin(),
                visible: !gap.contains(&time) && flow <= 3.,
            })
        });
        assert!(
            matches!(
                plan,
                PreviewPlan::Original {
                    reason: FallbackReason::VisibilityGap,
                    ..
                }
            ),
            "{gap:?}: {plan:?}"
        );
    }
}

#[test]
fn carrier_scale_zero_two_and_nonuniform_never_scale_attached_notes() {
    let res = resource();
    let mut chart = carrier_chart(1.);
    let expected = note_quad_at(&chart, 0, 0, &res, 9.2, 0.2);
    let expected_flow = solved(&chart, &res, 9.2);
    for (x, y) in [(0., 0.), (2., 2.), (0.1, 5.), (-3., 0.)] {
        chart.lines[0].object.scale.0 = core::AnimFloat::fixed(x);
        chart.lines[0].object.scale.1 = core::AnimFloat::fixed(y);
        assert_eq!(note_quad_at(&chart, 0, 0, &res, 9.2, 0.2), expected);
        assert_eq!(solved(&chart, &res, 9.2), expected_flow);
    }
}

#[test]
fn hold_head_body_and_tail_share_the_accepted_reversing_plan() {
    use phira_chart_play_tests::anim::Keyframe;
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].notes[0].height = 0.;
    chart.lines[0].notes[0].kind = NoteKind::Hold {
        end_time: 10.1,
        end_height: 0.2,
    };
    chart.lines[0].height = core::AnimFloat::new(vec![Keyframe::new(9.55, -0.145, 0), Keyframe::new(9.65, -0.5, 0), Keyframe::new(10., 0., 0)]);
    let plan = note_plan(&chart, 0, 0, &res, 0.45);
    assert!(matches!(plan, PreviewPlan::GateOnly { original_flow: 1., .. }));
    let quads = note_quad_at(&chart, 0, 0, &res, 9.65, plan.flow()).unwrap();
    assert!(quads.into_iter().all(quad_visible));
    assert!(plan.hides(false, 9.54));
    assert!(!plan.hides(false, 9.55));
}

#[test]
fn reversed_hold_body_cannot_fake_visibility_between_offscreen_head_and_tail() {
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].notes[0].height = 2.;
    chart.lines[0].notes[0].kind = NoteKind::Hold {
        end_time: 11.,
        end_height: -2.,
    };
    let quads = note_quad_at(&chart, 0, 0, &res, 9.55, 1.).unwrap();
    assert!(!geometry_visible(quads));
}

#[test]
fn fake_and_invalid_targets_do_not_run_a_solver_or_install_a_gate() {
    for (fake, start, reason) in [(true, 9., FallbackReason::FakeNote), (false, -0.1, FallbackReason::InvalidTarget)] {
        let plan = choose_plan(fake, 0.7, start, 10., 1., |_, _| panic!("unsupported note sampled"));
        assert_eq!(plan, PreviewPlan::original(0.7, reason));
        assert!(!plan.hides(fake, -100.));
    }
}

#[test]
fn renderer_owned_cover_and_negative_alpha_do_not_remove_travel_geometry() {
    let res = resource();
    let mut chart = carrier_chart(1.);
    chart.lines[0].notes[0].height = -0.1;
    chart.lines[0].show_below = false;
    assert!(note_quad_at(&chart, 0, 0, &res, 9.55, 1.).is_some());
    chart.lines[0].show_below = true;
    chart.lines[0].object.alpha = core::AnimFloat::fixed(-1.);
    assert!(note_quad_at(&chart, 0, 0, &res, 9.55, 1.).is_some());
    chart.lines[0].object.alpha = core::AnimFloat::fixed(-100.);
    assert!(!matches!(note_plan(&chart, 0, 0, &res, 0.45), PreviewPlan::Original { .. }));
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
fn transparent_note_or_control_keeps_the_same_geometric_flow_solution() {
    let res = resource();
    let mut chart = carrier_chart(1.);
    let expected = solved(&chart, &res, 9.2);
    chart.lines[0].notes[0].object.alpha = core::AnimFloat::fixed(0.);
    assert_eq!(solved(&chart, &res, 9.2), expected);
    chart.lines[0].notes[0].object.alpha = core::AnimFloat::fixed(1.);
    chart.lines[0].ctrl_obj.borrow_mut().alpha = core::AnimFloat::fixed(0.);
    assert_eq!(solved(&chart, &res, 9.2), expected);
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
