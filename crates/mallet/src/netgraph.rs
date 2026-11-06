//! A small performance overlay for the camera viewport, in the spirit of Source's
//! `net_graph`: frame rate, CPU time per frame, what culling drew, and a bar graph of
//! recent frames.

use std::collections::VecDeque;

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use crate::render3d::RenderStats;

/// What the renderer reports for the overlay besides frame times.
#[derive(Clone, Copy, Debug, Default)]
pub struct NetGraphExtra {
    pub stats: RenderStats,
    /// Viewports that showed their previous image instead of drawing again.
    pub reused: usize,
    /// Materials still loading in the background.
    pub loading: usize,
}

/// How many recent frames the graph shows.
const HISTORY: usize = 120;
const PANEL_SIZE: Vec2 = Vec2::new(236.0, 110.0);
const MARGIN: f32 = 6.0;
const GRAPH_HEIGHT: f32 = 40.0;
/// Frame time the graph's full height stands for, in milliseconds.
const GRAPH_RANGE_MS: f32 = 33.3;
/// Frame time of 60 frames per second, drawn as a reference line.
const TARGET_MS: f32 = 1000.0 / 60.0;
/// Colour of the drop shadow behind the overlay's text.
const SHADOW: Color32 = Color32::from_black_alpha(200);
/// Frames further apart than this were idle waits for input, not slow frames.
const IDLE_GAP_SECONDS: f32 = 0.25;

/// Recent frame timings.
#[derive(Default)]
pub struct NetGraph {
    /// CPU milliseconds of recent frames, oldest first.
    cpu_ms: VecDeque<f32>,
    /// Seconds between recent frames that followed each other without an idle wait.
    intervals: VecDeque<f32>,
}

impl NetGraph {
    /// Records one frame: its CPU time and the time since the frame before it.
    pub fn record(&mut self, cpu_ms: f32, interval_seconds: f32) {
        push_capped(&mut self.cpu_ms, cpu_ms);
        if interval_seconds > 0.0 && interval_seconds < IDLE_GAP_SECONDS {
            push_capped(&mut self.intervals, interval_seconds);
        }
    }

    /// The most recent back-to-back frame intervals in milliseconds, oldest first.
    pub fn recent_intervals(&self, count: usize) -> Vec<f32> {
        let skip = self.intervals.len().saturating_sub(count);
        self.intervals.iter().skip(skip).map(|seconds| seconds * 1000.0).collect()
    }

    /// Shortest, average and longest time between recent back-to-back frames, in
    /// milliseconds.
    pub fn interval_stats(&self) -> [f32; 3] {
        if self.intervals.is_empty() {
            return [0.0; 3];
        }
        let min = self.intervals.iter().copied().fold(f32::INFINITY, f32::min);
        let max = self.intervals.iter().copied().fold(0.0, f32::max);
        [min * 1000.0, average(&self.intervals) * 1000.0, max * 1000.0]
    }

    /// Draws the overlay in the bottom-right corner of `viewport`, straight over the scene
    /// with no background; a dark shadow keeps the text readable on bright textures.
    ///
    /// The frame rate counts only frames drawn back to back, because the editor redraws
    /// only when something changes. "max fps" is what the CPU time alone would allow.
    pub fn draw(&self, painter: &Painter, viewport: Rect, extra: &NetGraphExtra) {
        let stats = &extra.stats;
        let panel = Rect::from_min_size(viewport.max - PANEL_SIZE - Vec2::splat(MARGIN), PANEL_SIZE);
        if !viewport.contains_rect(panel) {
            return;
        }

        let last_cpu = self.cpu_ms.back().copied().unwrap_or(0.0);
        let average_cpu = average(&self.cpu_ms);
        let fps = match average(&self.intervals) {
            interval if interval > 0.0 => format!("{:.0}", 1.0 / interval),
            _ => "idle".to_owned(),
        };
        let max_fps = if average_cpu > 0.0 { 1000.0 / average_cpu } else { 0.0 };
        let left = panel.min + Vec2::new(6.0, 4.0);
        let lines = [
            format!("fps {fps:>4}   cpu {last_cpu:5.1} ms"),
            format!("avg {average_cpu:5.1} ms   max fps {max_fps:5.0}"),
            format!(
                "chunks {}/{}   tris {}",
                stats.chunks_drawn,
                stats.chunks,
                thousands(stats.triangles_drawn)
            ),
            if extra.loading > 0 {
                format!("reused {}/4   loading {} textures", extra.reused, extra.loading)
            } else {
                format!("reused {}/4", extra.reused)
            },
        ];
        for (row, line) in lines.into_iter().enumerate() {
            shadowed_text(painter, left + Vec2::new(0.0, 14.0 * row as f32), line);
        }

        let graph = Rect::from_min_max(
            Pos2::new(panel.min.x + 6.0, panel.max.y - 6.0 - GRAPH_HEIGHT),
            Pos2::new(panel.max.x - 6.0, panel.max.y - 6.0),
        );
        let bar_width = graph.width() / HISTORY as f32;
        let offset = HISTORY - self.cpu_ms.len();
        for (index, ms) in self.cpu_ms.iter().enumerate() {
            let height = (ms / GRAPH_RANGE_MS).min(1.0) * graph.height();
            let x = graph.min.x + (offset + index) as f32 * bar_width;
            let bar = Rect::from_min_max(Pos2::new(x, graph.max.y - height), Pos2::new(x + bar_width.max(1.0), graph.max.y));
            painter.rect_filled(bar, 0.0, bar_color(*ms));
        }
        let target_y = graph.max.y - (TARGET_MS / GRAPH_RANGE_MS) * graph.height();
        painter.line_segment(
            [Pos2::new(graph.min.x, target_y), Pos2::new(graph.max.x, target_y)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 90)),
        );
    }
}

fn shadowed_text(painter: &Painter, at: Pos2, text: String) {
    let font = FontId::monospace(11.0);
    painter.text(at + Vec2::splat(1.0), Align2::LEFT_TOP, &text, font.clone(), SHADOW);
    painter.text(at, Align2::LEFT_TOP, text, font, Color32::from_gray(230));
}

fn push_capped(history: &mut VecDeque<f32>, value: f32) {
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(value);
}

fn average(values: &VecDeque<f32>) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

/// Green below 8 ms, yellow up to a 60 fps frame, red above.
fn bar_color(ms: f32) -> Color32 {
    if ms < 8.0 {
        Color32::from_rgb(80, 200, 90)
    } else if ms < TARGET_MS {
        Color32::from_rgb(230, 200, 60)
    } else {
        Color32::from_rgb(230, 70, 60)
    }
}

/// Formats a count with thin separators, such as `160,800`.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
