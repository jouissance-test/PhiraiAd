//! Opt-in host audit hooks. The normal renderer submits every captured vertex;
//! no note position or visibility formula is replaced by this observer.
use crate::core::{Chart, Resource};
use macroquad::prelude::*;
use serde::Serialize;
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug, Serialize)]
pub struct Quad {
    pub line: usize,
    pub note: usize,
    /// Final camera clip coordinates after the Chart model reflection.
    pub ndc: [[f32; 2]; 4],
    pub rgba: [[u8; 4]; 4],
    pub uv: [[f32; 2]; 4],
}

#[derive(Default)]
struct Capture {
    enabled: bool,
    silent_audio: bool,
    tags: HashMap<usize, (usize, usize)>,
    chart_identity: usize,
    current: Option<(usize, usize)>,
    quads: Vec<Quad>,
}
thread_local! { static CAPTURE: RefCell<Capture> = RefCell::new(Capture::default()); }

pub fn enable_silent_audio() {
    CAPTURE.with(|c| c.borrow_mut().silent_audio = true);
}
pub(crate) fn silent_audio() -> bool {
    CAPTURE.with(|c| c.borrow().silent_audio)
}

pub(crate) fn capturing() -> bool {
    CAPTURE.with(|c| c.borrow().enabled)
}

/// Keep the receiver alive while Resource creates its three unused sound effects.
/// This backend is used only by the explicit audit executable, which never plays.
#[derive(Default)]
pub(crate) struct SilentBackend {
    setup: Option<sasa::backend::BackendSetup>,
}
impl sasa::Backend for SilentBackend {
    fn setup(&mut self, setup: sasa::backend::BackendSetup) -> anyhow::Result<()> {
        self.setup = Some(setup);
        Ok(())
    }
    fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn consume_broken(&self) -> bool {
        false
    }
}

pub fn begin(chart: &Chart) {
    CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        c.enabled = true;
        c.current = None;
        c.quads.clear();
        // Audit callers never change the chart's note storage during a scan.
        // Keep pointer tags between frames instead of rebuilding N entries at Hz.
        let identity = chart.lines.as_ptr() as usize;
        if c.chart_identity != identity {
            c.chart_identity = identity;
            c.tags.clear();
            for (line, carrier) in chart.lines.iter().enumerate() {
                for (note, value) in carrier.notes.iter().enumerate() {
                    c.tags.insert(value as *const _ as usize, (line, note));
                }
            }
        }
    });
}

pub fn finish() -> Vec<Quad> {
    CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        c.enabled = false;
        std::mem::take(&mut c.quads)
    })
}

/// Calls the exact production cache builder. The caller owns GameScene timing
/// and pause-state behavior; this API deliberately makes no claim to test it.
pub fn configure(chart: &mut Chart, res: &mut Resource, preview: Option<f64>) {
    chart.clear_fixed_preview();
    res.fixed_preview_chart_time = preview;
    res.config.fixed_preview_time_enabled = preview.is_some();
    if let Some(seconds) = preview {
        chart.rebuild_fixed_preview(res, seconds);
    }
}

pub(crate) fn set_note(address: usize) {
    CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        c.current = c.tags.get(&address).copied();
    });
}

pub(crate) fn record_quad(res: &Resource, vertices: &[Vertex; 4]) {
    CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        if !c.enabled {
            return;
        }
        let Some((line, note)) = c.current else {
            return;
        };
        let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
        let vertices: [([f32; 3], [f32; 2], [f32; 4]); 4] = vertices.map(Into::into);
        let ndc = vertices.map(|v| {
            let p = projection * vec4(if res.config.flip_x() { -v.0[0] } else { v.0[0] }, -v.0[1], 0., 1.);
            [p.x / p.w, p.y / p.w]
        });
        c.quads.push(Quad {
            line,
            note,
            ndc,
            rgba: vertices.map(|v| v.2.map(|c| (c * 255.).round() as u8)),
            uv: vertices.map(|v| v.1),
        });
    });
}
