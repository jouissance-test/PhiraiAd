//! Exercise the production GameScene cache key and sync method with a chart
//! adapter that records cache clears and rebuild horizons.
#![allow(dead_code)]

use anyhow::Result;
use phira_chart_play_tests::{config, practice_view};
use std::{cell::Cell, ops::Range, rc::Rc};
#[path = "../../../prpr/src/time.rs"]
mod time;
use time::TimeManager;

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{}};
}
mod tracing {
    pub use crate::info;
}

#[derive(PartialEq, Eq)]
enum GameMode {
    Normal,
    TweakOffset,
    EditChartPlay,
    Exercise,
    NoRetry,
    View,
}
enum State {
    Playing,
    BeforeMusic,
}
enum TouchDebugClearSource {
    PauseButton,
    AutoFlipTransition,
}
struct Judge {
    clears: usize,
}
impl Judge {
    fn clear_touch_input_with_source(&mut self, _source: TouchDebugClearSource) {
        self.clears += 1;
    }
}
struct Music {
    at: f64,
    paused: bool,
    pitch: bool,
}
impl Music {
    fn paused(&mut self) -> bool {
        self.paused
    }
    fn pause(&mut self) -> Result<()> {
        self.paused = true;
        Ok(())
    }
    fn play(&mut self) -> Result<()> {
        self.paused = false;
        Ok(())
    }
    fn position(&self) -> f64 {
        self.at
    }
    fn seek_to(&mut self, time: f64) -> Result<()> {
        self.at = time;
        Ok(())
    }
    fn preserves_pitch(&self) -> bool {
        self.pitch
    }
}

struct ResPackInfo {
    hold_compact: bool,
}
struct ResPack {
    info: ResPackInfo,
}
struct Resource {
    config: config::Config,
    replay_view: bool,
    aspect_ratio: f32,
    last_vp: (i32, i32, i32, i32),
    practice_view: practice_view::PracticeView,
    note_width: f32,
    note_flow_speed: f32,
    res_pack: ResPack,
    fixed_preview_chart_time: Option<f64>,
    time: f64,
}

#[derive(Default)]
struct RebuildStats {
    solved: usize,
    fallback: usize,
}
#[derive(Default)]
struct Chart {
    rebuild_horizons: Vec<f64>,
    clear_count: usize,
    offset: f64,
}
impl Chart {
    fn rebuild_fixed_preview(&mut self, _res: &Resource, preview_chart_seconds: f64) -> RebuildStats {
        self.rebuild_horizons.push(preview_chart_seconds);
        RebuildStats::default()
    }

    fn clear_fixed_preview(&mut self) {
        self.clear_count += 1;
    }
}

struct GameScene {
    mode: GameMode,
    res: Resource,
    chart: Chart,
    fixed_preview_key: Option<FixedPreviewKey>,
    music: Music,
    judge: Judge,
    state: State,
    exercise_range: Range<f64>,
    exercise_state_reset_pending: bool,
    pause_rewind: Option<f64>,
    reset: Option<f64>,
    correct_sound_events: Option<()>,
    info_offset: f64,
}
impl GameScene {
    fn new_music(res: &mut Resource, mode: &GameMode, _events: &Option<()>, _offset: f64) -> Result<Music> {
        Ok(Music {
            at: 0.,
            paused: true,
            pitch: res.config.preserve_pitch_for(*mode == GameMode::Exercise),
        })
    }
    fn begin_armed_replay(&mut self) {}
    fn reset_exercise_judgement_to(&mut self, time: f64) {
        self.reset = Some(time);
    }
}

include!(concat!(env!("OUT_DIR"), "/fixed_preview_hot_reload.rs"));
include!(concat!(env!("OUT_DIR"), "/pause_resume.rs"));

fn scene() -> GameScene {
    let mut config = config::Config::default();
    config.fixed_preview_time_enabled = true;
    config.fixed_preview_time = 0.4;
    GameScene {
        mode: GameMode::Normal,
        res: Resource {
            config,
            replay_view: false,
            aspect_ratio: 16. / 9.,
            last_vp: (0, 0, 1920, 1080),
            practice_view: practice_view::PracticeView::default(),
            note_width: 1.,
            note_flow_speed: 1.,
            res_pack: ResPack {
                info: ResPackInfo { hold_compact: false },
            },
            fixed_preview_chart_time: None,
            time: 5.,
        },
        chart: Chart::default(),
        fixed_preview_key: None,
        music: Music {
            at: 5.,
            paused: false,
            pitch: false,
        },
        judge: Judge { clears: 0 },
        state: State::Playing,
        exercise_range: 0. ..10.,
        exercise_state_reset_pending: false,
        pause_rewind: None,
        reset: None,
        correct_sound_events: None,
        info_offset: 0.,
    }
}

#[test]
fn paused_setting_edits_invalidate_key_and_rebuild_at_real_preview_times_speed() {
    let mut game = scene();
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 1);
    assert!((game.chart.rebuild_horizons[0] - 0.4).abs() < 1e-7);
    assert!((game.res.fixed_preview_chart_time.unwrap() - 0.4).abs() < 1e-7);

    // Pause the production playback path. The clock must stay frozen while the
    // paused menu changes the playback speed and real preview duration.
    let wall = Rc::new(Cell::new(0.));
    let wall_clock = wall.clone();
    let mut clock = TimeManager::manual(Box::new(move || wall_clock.get()));
    clock.seek_to(5.);
    GameScene::pause_playback(&mut game.music, &mut clock, &mut game.judge, TouchDebugClearSource::PauseButton).unwrap();
    wall.set(0.8);
    assert!(clock.paused());
    assert_eq!(clock.now(), 5.);

    // The paused menu changes both settings before the next scene update.
    game.res.config.speed = 0.5;
    game.res.config.fixed_preview_time = 0.6;
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 2);
    let expected = game.res.config.fixed_preview_seconds() as f64 * game.res.config.speed as f64;
    assert!((game.chart.rebuild_horizons[1] - expected).abs() < 1e-12);
    assert_eq!(game.res.fixed_preview_chart_time, Some(expected));
    assert_eq!(clock.now(), 5.);

    // Resume through the production path, adopting the new speed. The preview
    // cache key remains current and therefore is not rebuilt a second time.
    game.resume_from_pause(&mut clock, None).unwrap();
    assert!(!clock.paused());
    assert_eq!(clock.speed, 0.5);
    assert_eq!(game.chart.rebuild_horizons.len(), 2);
    wall.set(1.8);
    assert!((clock.now() - 2.5).abs() < 1e-10);

    // An unchanged key is cached and does not rebuild on subsequent frames.
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 2);
}

#[test]
fn disabling_clears_cache_and_reenabling_rebuilds_it() {
    let mut game = scene();
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 1);

    game.res.config.fixed_preview_time_enabled = false;
    game.sync_fixed_preview();
    assert_eq!(game.chart.clear_count, 1);
    assert_eq!(game.res.fixed_preview_chart_time, None);

    // A stable disabled key clears once, then remains a no-op.
    game.sync_fixed_preview();
    assert_eq!(game.chart.clear_count, 1);

    game.res.config.fixed_preview_time_enabled = true;
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 2);
    assert_eq!(game.chart.clear_count, 1);
    assert!((game.res.fixed_preview_chart_time.unwrap() - 0.4).abs() < 1e-7);
}

#[test]
fn changing_effective_original_flow_invalidates_all_preview_plans() {
    let mut game = scene();
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 1);
    game.res.note_flow_speed = 2.;
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 2);
    game.sync_fixed_preview();
    assert_eq!(game.chart.rebuild_horizons.len(), 2);
}
