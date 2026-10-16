use anyhow::{bail, Context, Result};
use macroquad::miniquad;
use macroquad::prelude::*;
use prpr::{
    core::Resource,
    fs,
    render_audit::{self, Quad},
    scene::GameScene,
    ui::{FontArc, TextPainter, Ui},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufWriter, Read, Write},
    path::PathBuf,
};

#[derive(Debug)]
struct Options {
    chart: PathBuf,
    out: PathBuf,
    assets: PathBuf,
    preview: f64,
    speed: f64,
    start: f64,
    end: Option<f64>,
    hz: f64,
    width: i32,
    height: i32,
    frames: Option<PathBuf>,
    scenario: Vec<Event>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Event {
    at: f64,
    speed: Option<f64>,
    preview: Option<f64>,
    #[serde(default)]
    pause_real_seconds: f64,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut values = BTreeMap::new();
        let mut args = std::env::args().skip(1);
        while let Some(key) = args.next() {
            if key == "--help" {
                println!("phira-preview-audit --chart ZIP --out JSON [--preview 0.45 --speed 1 --start 0 --end 180 --hz 120 --assets DIR --width 1600 --height 900 --frames JSONL --scenario FILE.json]");
                std::process::exit(0);
            }
            if ![
                "--chart",
                "--out",
                "--preview",
                "--speed",
                "--start",
                "--end",
                "--hz",
                "--assets",
                "--width",
                "--height",
                "--frames",
                "--scenario",
            ]
            .contains(&key.as_str())
            {
                bail!("Unknown option {key}");
            }
            values.insert(key.clone(), args.next().with_context(|| format!("Missing value for {key}"))?);
        }
        let number = |key: &str, default: f64| -> Result<f64> { Ok(values.get(key).map(|s| s.parse::<f64>()).transpose()?.unwrap_or(default)) };
        let chart = values.get("--chart").context("--chart is required")?.into();
        let out = values.get("--out").context("--out is required")?.into();
        let assets = values
            .get("--assets")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
        let preview = number("--preview", 0.45)?;
        let speed = number("--speed", 1.)?;
        let start = number("--start", 0.)?;
        let hz = number("--hz", 120.)?;
        let width = number("--width", 1600.)? as i32;
        let height = number("--height", 900.)? as i32;
        let end = values.get("--end").map(|s| s.parse::<f64>()).transpose()?;
        let scenario: Vec<Event> = values
            .get("--scenario")
            .map(|p| -> Result<_> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) })
            .transpose()?
            .unwrap_or_default();
        anyhow::ensure!(
            preview.is_finite()
                && preview > 0.
                && speed.is_finite()
                && speed > 0.
                && hz.is_finite()
                && hz > 0.
                && start.is_finite()
                && start >= 0.
                && width > 0
                && height > 0,
            "Invalid numeric argument"
        );
        anyhow::ensure!(end.is_none_or(|t| t.is_finite() && t > start), "--end must follow --start");
        for (i, e) in scenario.iter().enumerate() {
            anyhow::ensure!(
                e.at.is_finite()
                    && e.at >= start
                    && e.speed.is_none_or(|v| v.is_finite() && v > 0.)
                    && e.preview.is_none_or(|v| v.is_finite() && v > 0.)
                    && e.pause_real_seconds.is_finite()
                    && e.pause_real_seconds >= 0.,
                "Invalid scenario event {i}"
            );
            anyhow::ensure!(i == 0 || scenario[i - 1].at <= e.at, "Scenario events must be ordered");
        }
        Ok(Self {
            chart,
            out,
            assets,
            preview,
            speed,
            start,
            end,
            hz,
            width,
            height,
            frames: values.get("--frames").map(PathBuf::from),
            scenario,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct Sample {
    chart_time: f64,
    real_time: f64,
    active_real_time: f64,
    settings_segment: usize,
    quads: Vec<Quad>,
    center: [f32; 2],
}
#[derive(Default, Serialize)]
struct NoteStats {
    line: usize,
    note: usize,
    visible_frames: usize,
    submitted_frames: usize,
    quad_count: usize,
    spans: Vec<[f64; 2]>,
    spans_real: Vec<[f64; 2]>,
    longest_contiguous_visible_real_seconds: f64,
    first_sample: Option<Sample>,
    last_sample: Option<Sample>,
    max_center_step_ndc: f32,
    max_center_step_per_real_second: f64,
    max_step_samples: Option<[Sample; 2]>,
    max_speed_samples: Option<[Sample; 2]>,
    transition_jumps: Vec<Value>,
    max_transition_step_ndc: f32,
    topology_changes: usize,
    #[serde(skip)]
    previous_frame: Option<usize>,
}

/// Independent distance and velocity peaks. Scenario boundaries are explicit
/// discontinuities and never compete with continuous motion measurements.
fn record_motion(state: &mut NoteStats, sample: &Sample, contiguous: bool) {
    let Some(previous) = state.last_sample.as_ref().filter(|_| contiguous) else {
        return;
    };
    let step = ((sample.center[0] - previous.center[0]).powi(2) + (sample.center[1] - previous.center[1]).powi(2)).sqrt();
    let active_dt = sample.active_real_time - previous.active_real_time;
    if previous.quads.len() != sample.quads.len() {
        state.topology_changes += 1;
    }
    if previous.settings_segment != sample.settings_segment {
        state.max_transition_step_ndc = state.max_transition_step_ndc.max(step);
        state.transition_jumps.push(json!({"from":previous,"to":sample,"step_ndc":step,"elapsed_real_seconds":sample.real_time-previous.real_time,"active_real_seconds":active_dt,"from_segment":previous.settings_segment,"to_segment":sample.settings_segment}));
        return;
    }
    if step > state.max_center_step_ndc {
        state.max_center_step_ndc = step;
        state.max_step_samples = Some([previous.clone(), sample.clone()]);
    }
    if active_dt > 0. {
        let speed = step as f64 / active_dt;
        if speed > state.max_center_step_per_real_second {
            state.max_center_step_per_real_second = speed;
            state.max_speed_samples = Some([previous.clone(), sample.clone()]);
        }
    }
}

fn calibrated_summary(calibration: &Value, modes: &BTreeMap<&str, Value>, note_count: usize, solver: &[Value], segments: usize) -> Result<Value> {
    anyhow::ensure!(calibration.get("passed").and_then(Value::as_bool) == Some(true), "Refusing summary/report: renderer calibration did not pass");
    let mut reason_counts = BTreeMap::<&str, usize>::new();
    for note in solver {
        *reason_counts.entry(note["reason"].as_str().unwrap_or("unknown")).or_default() += 1;
    }
    let mut mode_summary = BTreeMap::new();
    for (name, mode) in modes {
        let notes = mode["notes"].as_array().context("Invalid mode notes for summary")?;
        let visible_notes = notes.iter().filter(|n| n["visible_frames"].as_u64().unwrap_or(0) > 0).count();
        mode_summary.insert(name, json!({"frames":mode["frames"],"submitted_note_entries":notes.len(),"visible_notes":visible_notes,"notes_without_visible_samples":note_count-visible_notes,"visible_note_frames":notes.iter().map(|n|n["visible_frames"].as_u64().unwrap_or(0)).sum::<u64>(),"transition_jump_count":notes.iter().map(|n|n["transition_jumps"].as_array().map_or(0,Vec::len)).sum::<usize>()}));
    }
    Ok(
        json!({"calibration_passed":true,"pre_hit_only":true,"note_count":note_count,"initial_solver_reason_counts":reason_counts,"solver_segments_count":segments,"modes":mode_summary}),
    )
}

// Independent polygon clipping of the submitted quads. This does not call the
// fixed-preview sampler or its quad_visible predicate.
fn visible(q: &Quad) -> bool {
    if q.rgba.iter().all(|c| c[3] == 0) || q.ndc.iter().flatten().any(|v| !v.is_finite()) {
        return false;
    }
    let mut polygon = q.ndc.to_vec();
    for (axis, bound, sign) in [(0, -1., 1.), (0, 1., -1.), (1, -1., 1.), (1, 1., -1.)] {
        let input = std::mem::take(&mut polygon);
        if input.is_empty() {
            return false;
        }
        let mut p = *input.last().unwrap();
        for c in input {
            let pin = (p[axis] - bound) * sign >= 0.;
            let cin = (c[axis] - bound) * sign >= 0.;
            if pin != cin {
                let t = (bound - p[axis]) / (c[axis] - p[axis]);
                polygon.push([p[0] + t * (c[0] - p[0]), p[1] + t * (c[1] - p[1])]);
            }
            if cin {
                polygon.push(c);
            }
            p = c;
        }
    }
    polygon.len() >= 3
        && (0..polygon.len())
            .map(|i| {
                let a = polygon[i];
                let b = polygon[(i + 1) % polygon.len()];
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f32>()
            .abs()
            > 1e-12
}

fn file_identity(path: &std::path::Path) -> Result<Value> {
    let mut file = std::fs::File::open(path)?;
    let bytes = file.metadata()?.len();
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 65536];
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        hash.update(&chunk[..count]);
    }
    Ok(json!({"path":path,"bytes":bytes,"sha256":format!("{:x}",hash.finalize())}))
}

fn capture_frame(
    chart: &mut prpr::core::Chart,
    res: &mut Resource,
    painter: &mut TextPainter,
    time: f64,
    viewport: (i32, i32, i32, i32),
) -> Vec<Quad> {
    res.time = time;
    chart.update(res);
    set_camera(&res.camera);
    clear_background(BLACK);
    render_audit::begin(chart);
    chart.render(&mut Ui::new(painter, Some(viewport)), res, None);
    let result = render_audit::finish();
    unsafe { get_internal_gl() }.flush();
    result
}

/// A known constant-speed chart calibrates the *independent production drawing*
/// before any corpus result is trusted. Coordinates are analytic for this tiny
/// fixture; no fixed_preview geometry sampler supplies expected positions.
fn calibrate(res: &mut Resource, painter: &mut TextPainter, viewport: (i32, i32, i32, i32)) -> Result<Value> {
    let source = json!({"formatVersion":3,"offset":0,"judgeLineList":[{"bpm":120,"notesAbove":[{"type":1,"time":128,"positionX":1,"holdTime":0,"speed":1,"floorPosition":2}],"notesBelow":[],"speedEvents":[{"startTime":0,"endTime":256,"value":1}],"judgeLineDisappearEvents":[{"startTime":0,"endTime":256,"start":1,"end":1}],"judgeLineRotateEvents":[{"startTime":0,"endTime":256,"start":0,"end":0}],"judgeLineMoveEvents":[{"startTime":0,"endTime":256,"start":0.5,"end":0.5,"start2":0.5,"end2":0.5}]}]});
    let mut chart = prpr::parse::parse_phigros(&source.to_string(), Default::default())?;
    let original_flow = res.note_flow_speed;
    res.note_flow_speed = 1.;
    render_audit::configure(&mut chart, res, None);
    let center = |quads: &[Quad]| -> Result<[f32; 2]> {
        let q = quads.first().context("Calibration fixture did not render")?;
        Ok([q.ndc.iter().map(|p| p[0]).sum::<f32>() / 4., q.ndc.iter().map(|p| p[1]).sum::<f32>() / 4.])
    };
    let baseline = capture_frame(&mut chart, res, painter, 1.75, viewport);
    let base_center = center(&baseline)?;
    let expected = [0.1125, (0.25 / 0.83175) as f32];
    anyhow::ensure!(
        (base_center[0] - expected[0]).abs() < 2e-5 && (base_center[1] - expected[1]).abs() < 2e-5,
        "Renderer coordinate calibration failed: observed {base_center:?}, expected {expected:?}"
    );
    res.note_flow_speed = 2.;
    let doubled = capture_frame(&mut chart, res, painter, 1.75, viewport);
    let double_center = center(&doubled)?;
    anyhow::ensure!((double_center[1] - base_center[1] * 2.).abs() < 2e-5, "Renderer flow calibration failed");
    res.config.mods.insert(prpr::config::Mods::FLIP_X);
    let mirrored = capture_frame(&mut chart, res, painter, 1.75, viewport);
    let mirror_center = center(&mirrored)?;
    anyhow::ensure!((mirror_center[0] + base_center[0]).abs() < 2e-5, "Renderer reflection calibration failed");
    res.config.mods.remove(prpr::config::Mods::FLIP_X);
    res.note_flow_speed = 1.;
    render_audit::configure(&mut chart, res, Some(0.45));
    let before = capture_frame(&mut chart, res, painter, 1.54, viewport);
    let after = capture_frame(&mut chart, res, painter, 1.56, viewport);
    anyhow::ensure!(
        before.is_empty() && after.iter().any(visible),
        "Fixed preview gate/calibrated flow did not produce the expected hidden/visible boundary"
    );
    render_audit::configure(&mut chart, res, None);
    res.note_flow_speed = original_flow;
    res.time = 0.;
    Ok(
        json!({"passed":true,"fixture":"production PGR parser; one Tap at chart_time=2; constant speed=1; centered static line","sample_chart_time":1.75,"expected_baseline_center":expected,"observed_baseline_center":base_center,"observed_double_flow_center":double_center,"observed_mirrored_center":mirror_center,"baseline_quads":baseline,"fixed_after_gate_quads":after,"checks":["analytic baseline position","flow multiplier doubles travel","Chart reflection reaches final NDC","fixed horizon hides before target and production render appears after target"]}),
    )
}

async fn run(options: Options) -> Result<()> {
    render_audit::enable_silent_audio();
    macroquad::file::set_pc_assets_folder(&options.assets.to_string_lossy());
    let chart_bytes = std::fs::read(&options.chart)?;
    let chart_hash = format!("{:x}", Sha256::digest(&chart_bytes));
    let mut chart_fs = fs::fs_from_file(&options.chart)?;
    let info = fs::load_info(chart_fs.as_mut()).await?;
    let (mut chart, _, format) = GameScene::load_chart(chart_fs.as_mut(), &info).await?;
    let end = options.end.unwrap_or_else(|| {
        chart
            .lines
            .iter()
            .flat_map(|l| &l.notes)
            .map(|n| match n.kind {
                prpr::core::NoteKind::Hold { end_time, .. } => end_time,
                _ => n.time,
            })
            .fold(0., f64::max)
            + 1.
    });
    let mut config = prpr::config::Config::default();
    config.speed = options.speed as f32;
    config.sample_count = 1;
    config.disable_effect = true;
    config.fxaa = false;
    config.particle = false;
    config.replay_recording_enabled = false;
    let white = Texture2D::from_rgba8(1, 1, &[255; 4]);
    let mut res = Resource::new(config, info, chart_fs, None, white.into(), white.into(), true).await?;
    res.update_size((0, 0, options.width, options.height));
    let font = FontArc::try_from_vec(std::fs::read(options.assets.join("font.ttf"))?).map_err(|e| anyhow::anyhow!("font: {e}"))?;
    let mut painter = TextPainter::new(font, None);
    let calibration = calibrate(&mut res, &mut painter, (0, 0, options.width, options.height))?;
    let note_metadata: Vec<_> = chart
        .lines
        .iter()
        .enumerate()
        .flat_map(|(li, l)| {
            l.notes
                .iter()
                .enumerate()
                .map(move |(ni, n)| json!({"line":li,"note":ni,"time":n.time,"fake":n.fake,"kind":format!("{:?}",n.kind),"above":n.above}))
        })
        .collect();
    let solver = chart.audit_preview_solver(&res, options.preview * res.config.speed as f64);
    let mut solver_segments = vec![
        json!({"segment_index":0,"applied_chart_time":options.start,"preview_real_seconds":options.preview,"speed":res.config.speed,"preview_chart_seconds":options.preview*res.config.speed as f64,"solver":solver}),
    ];
    let mut modes = BTreeMap::new();
    let mut frame_file = options.frames.as_ref().map(std::fs::File::create).transpose()?.map(BufWriter::new);
    let mut event_log = Vec::new();
    for fixed in [false, true] {
        let name = if fixed { "fixed" } else { "baseline" };
        chart.reset();
        res.config.speed = options.speed as f32;
        let mut preview = options.preview;
        let initial_preview_chart = preview * res.config.speed as f64;
        render_audit::configure(&mut chart, &mut res, fixed.then_some(initial_preview_chart));
        let mut stats: BTreeMap<(usize, usize), NoteStats> = BTreeMap::new();
        let mut chart_time = options.start;
        let mut real_time = 0.;
        let mut frame = 0;
        let mut event_index = 0;
        while chart_time <= end + 1e-9 {
            while let Some(event) = options.scenario.get(event_index).filter(|e| e.at <= chart_time + 1e-9) {
                let old_speed = res.config.speed;
                if let Some(v) = event.speed {
                    res.config.speed = v as f32;
                }
                if let Some(v) = event.preview {
                    preview = v;
                }
                real_time += event.pause_real_seconds;
                let preview_chart = preview * res.config.speed as f64;
                render_audit::configure(&mut chart, &mut res, fixed.then_some(preview_chart));
                if fixed {
                    solver_segments.push(json!({"segment_index":event_index+1,"applied_chart_time":chart_time,"preview_real_seconds":preview,"speed":res.config.speed,"preview_chart_seconds":preview_chart,"solver":chart.audit_preview_solver(&res,preview_chart)}));
                }
                event_log.push(json!({"mode":name,"event":event,"applied_chart_time":chart_time,"real_time":real_time,"old_speed":old_speed,"new_speed":res.config.speed,"preview_chart_seconds":preview*res.config.speed as f64,"coverage":"render_cache_rebuild_only"}));
                event_index += 1;
            }
            res.time = chart_time;
            chart.update(&mut res);
            set_camera(&res.camera);
            clear_background(BLACK);
            render_audit::begin(&chart);
            let mut ui = Ui::new(&mut painter, Some((0, 0, options.width, options.height)));
            chart.render(&mut ui, &mut res, None);
            let quads = render_audit::finish();
            if let Some(file) = &mut frame_file {
                serde_json::to_writer(
                    &mut *file,
                    &json!({"mode":name,"frame":frame,"chart_time":chart_time,"real_time":real_time,"speed":res.config.speed,"preview_real_seconds":preview,"quads":quads}),
                )?;
                writeln!(file)?;
            }
            let mut grouped: BTreeMap<(usize, usize), Vec<Quad>> = BTreeMap::new();
            for quad in quads {
                grouped.entry((quad.line, quad.note)).or_default().push(quad);
            }
            for ((line, note), quads) in grouped {
                // Preview diagnostics concern approach motion only. NotJudged
                // notes may keep drawing after hit, but cannot pollute metrics.
                if chart_time > chart.lines[line].notes[note].time + 1e-9 {
                    continue;
                }
                let state = stats.entry((line, note)).or_insert_with(|| NoteStats {
                    line,
                    note,
                    ..Default::default()
                });
                state.submitted_frames += 1;
                state.quad_count += quads.len();
                if !quads.iter().any(visible) {
                    continue;
                }
                let center = [
                    quads[0].ndc.iter().map(|p| p[0]).sum::<f32>() / 4.,
                    quads[0].ndc.iter().map(|p| p[1]).sum::<f32>() / 4.,
                ];
                let sample = Sample {
                    chart_time,
                    real_time,
                    active_real_time: frame as f64 / options.hz,
                    settings_segment: event_index,
                    quads,
                    center,
                };
                let contiguous = state.previous_frame.is_some_and(|f| f + 1 == frame);
                if !contiguous {
                    state.spans.push([chart_time, chart_time]);
                    state.spans_real.push([real_time, real_time]);
                } else {
                    state.spans.last_mut().unwrap()[1] = chart_time;
                    state.spans_real.last_mut().unwrap()[1] = real_time;
                }
                let real_span = state.spans_real.last().unwrap();
                state.longest_contiguous_visible_real_seconds = state.longest_contiguous_visible_real_seconds.max(real_span[1] - real_span[0]);
                record_motion(state, &sample, contiguous);
                state.visible_frames += 1;
                state.previous_frame = Some(frame);
                if state.first_sample.is_none() {
                    state.first_sample = Some(sample.clone());
                }
                state.last_sample = Some(sample);
            }
            // Production line/note/batch geometry has now been submitted and
            // observed. Pixel rasterization is not an oracle for this geometric
            // scan; discard the queued GL commands instead of rendering 80k
            // full viewports through a software Mesa driver. Calibration above
            // does rasterize its known fixture with flush().
            unsafe { get_internal_gl() }.quad_gl.clear_draw_calls();
            if frame % 600 == 0 {
                eprintln!("{name}: chart={chart_time:.3}/{end:.3} frame={frame}");
            }
            frame += 1;
            chart_time += res.config.speed as f64 / options.hz;
            real_time += 1. / options.hz;
        }
        modes.insert(name, json!({"frames":frame,"sampled_real_seconds":real_time,"notes":stats.into_values().collect::<Vec<_>>()}));
    }
    if let Some(file) = &mut frame_file {
        file.flush()?;
    }
    let summary = calibrated_summary(&calibration, &modes, note_metadata.len(), &solver, solver_segments.len())?;
    let report = json!({"schema_version":1,"calibration":calibration,"metadata":{"chart":options.chart,"chart_sha256":chart_hash,"chart_format":format!("{:?}",format),"chart_name":res.info.name,"viewport":[options.width,options.height],"effective_aspect_ratio":res.aspect_ratio,"preview_real_seconds":options.preview,"speed":options.speed,"preview_chart_seconds":options.preview*options.speed,"start_chart_time":options.start,"end_chart_time":end,"hz_real":options.hz,"scenario":options.scenario,"metrics_scope":"pre_hit_only: chart_time <= note.time; raw --frames includes all submitted quads","renderer":"production parse -> Chart::update -> JudgeLine::render -> Note::render -> draw_tex_pts -> NoteBuffer::push; WSLg hidden OpenGL","coverage_limitations":["No judgement/input/audio playback: notes retain NotJudged status; aggregate note metrics stop at hit time.","No shader/postprocessing/noise-area framebuffer or texture-alpha pixel readback; geometric visibility clips actual submitted alpha-nonzero quads.","Per-note capture excludes judge-line sprites, particles, HUD and bad-note effects. PracticeView and chart_post_view remain defaults and are not exercised.","Scenario events test render-cache rebuilding only; GameScene pause/resume state machine and its cache invalidation trigger are not exercised. Pauses add real-time duration without re-rendering frozen frames.","Fixed-time samples may miss sub-frame appearances; bounds use chart time, hz is real-time, preview_chart=preview_real*speed. Visibility spans touching scan bounds are censored.","Center-step values are candidates, not proof of bugs; Hold body topology can change naturally."]},"notes":note_metadata,"solver":solver,"scenario_events":event_log,"modes":modes});
    let mut report = report;
    report["solver_segments"] = json!(solver_segments);
    report["summary"] = summary;
    report["metadata"]["calibration_passed"] = json!(true);
    report["metadata"]["summary_valid"] = json!(true);
    let mut pack_files: Vec<_> = std::fs::read_dir(options.assets.join("respack"))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    pack_files.sort();
    let pack_manifest: Vec<_> = pack_files
        .iter()
        .filter(|p| p.is_file())
        .map(|p| file_identity(p))
        .collect::<Result<_>>()?;
    report["metadata"]["assets_root"] = json!(std::fs::canonicalize(&options.assets)?);
    report["metadata"]["chart_viewport"] = json!(res.camera.viewport);
    report["metadata"]["baseline_note_flow_speed"] = json!(res.note_flow_speed);
    report["metadata"]["note_width"] = json!(res.note_width);
    report["metadata"]["resource_pack_manifest"] = json!(pack_manifest);
    report["metadata"]["engine"] = file_identity(&std::env::current_exe()?)?;
    report["metadata"]["rasterization"] = json!("Calibration submits to OpenGL; scan samples execute production geometry and discard queued GPU draws after capture. No per-frame pixel readback.");
    report["metadata"]["real_time_origin"] = json!("0 at scan start; includes injected pause duration");
    if let Some(parent) = options.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = BufWriter::new(std::fs::File::create(&options.out)?);
    serde_json::to_writer_pretty(&mut output, &report)?;
    output.flush()?;
    eprintln!("Report written: {}", options.out.display());
    Ok(())
}

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime for production resource loading");
    let _runtime_guard = runtime.enter();
    let options = match Options::parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(2);
        }
    };
    let conf = miniquad::conf::Conf {
        window_title: "Phira renderer audit".into(),
        window_width: options.width,
        window_height: options.height,
        headless: true,
        ..Default::default()
    };
    macroquad::Window::from_config(conf, async move {
        // GL remains on this thread. Tokio owns the waker for production image
        // decode tasks; handing those tasks macroquad's panic-on-wake waker is
        // unsupported even though their results can eventually be polled.
        match tokio::runtime::Handle::current().block_on(run(options)) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Audit failed: {e:#}");
                std::process::exit(1)
            }
        }
    });
}

#[cfg(test)]
mod metric_tests {
    use super::*;

    fn sample(center: f32, elapsed: f64, active: f64, segment: usize) -> Sample {
        Sample {
            chart_time: active,
            real_time: elapsed,
            active_real_time: active,
            settings_segment: segment,
            quads: Vec::new(),
            center: [center, 0.],
        }
    }
    fn advance(state: &mut NoteStats, next: Sample) {
        record_motion(state, &next, true);
        state.last_sample = Some(next);
    }

    #[test]
    fn smaller_step_can_set_independent_peak_velocity() {
        let mut state = NoteStats::default();
        state.last_sample = Some(sample(0., 0., 0., 0));
        advance(&mut state, sample(1., 100., 100., 0));
        advance(&mut state, sample(1.2, 100.01, 100.01, 0));
        assert_eq!(state.max_center_step_ndc, 1.);
        assert!((state.max_center_step_per_real_second - 20.).abs() < 1e-4);
        assert_eq!(state.max_step_samples.as_ref().unwrap()[1].center[0], 1.);
        assert_eq!(state.max_speed_samples.as_ref().unwrap()[1].center[0], 1.2);
    }

    #[test]
    fn pause_setting_transition_is_separate_and_velocity_uses_active_clock() {
        let mut state = NoteStats::default();
        state.last_sample = Some(sample(0., 0., 0., 0));
        advance(&mut state, sample(1., 100.1, 0.1, 1));
        assert_eq!(state.max_center_step_ndc, 0.);
        assert_eq!(state.max_center_step_per_real_second, 0.);
        assert_eq!(state.transition_jumps.len(), 1);
        assert_eq!(state.max_transition_step_ndc, 1.);
        advance(&mut state, sample(1.2, 100.2, 0.2, 1));
        advance(&mut state, sample(1.3, 100.21, 0.21, 1));
        assert!((state.max_center_step_ndc - 0.2).abs() < 1e-6);
        assert!((state.max_center_step_per_real_second - 10.).abs() < 1e-4);
        assert_eq!(state.max_speed_samples.as_ref().unwrap()[1].settings_segment, 1);
        // Elapsed time includes an extra injected freeze; active dt still wins.
        advance(&mut state, sample(1.5, 150.22, 0.22, 1));
        assert!((state.max_center_step_per_real_second - 20.).abs() < 1e-4);
    }

    #[test]
    fn missing_frame_never_creates_continuous_or_transition_motion() {
        let mut state = NoteStats::default();
        state.last_sample = Some(sample(0., 0., 0., 0));
        record_motion(&mut state, &sample(10., 2., 2., 1), false);
        assert_eq!(state.max_center_step_ndc, 0.);
        assert!(state.transition_jumps.is_empty());
    }

    #[test]
    fn summary_fails_closed_without_explicit_successful_calibration() {
        for calibration in [json!({"passed":false}), json!({}), Value::Null] {
            assert!(calibrated_summary(&calibration, &BTreeMap::new(), 0, &[], 1).is_err());
        }
        assert_eq!(calibrated_summary(&json!({"passed":true}), &BTreeMap::new(), 0, &[], 1).unwrap()["calibration_passed"], true);
    }
}
