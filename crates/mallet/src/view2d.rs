//! Orthographic 2D viewports (top, front and side), drawn the way Hammer draws them.

use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Response};
use glam::{DVec2, DVec3};
use mallet_vmf::geometry::Bounds;

use crate::canvas::Canvas;
use crate::scene::Scene;

const MIN_SCALE: f64 = 1.0 / 256.0;
const MAX_SCALE: f64 = 32.0;
/// Grid lines closer than this many pixels are thinned out by doubling their spacing.
const MIN_GRID_PIXELS: f64 = 4.0;
const MAX_GRID_LINES: f64 = 4000.0;
/// Scroll distance egui reports for one mouse wheel notch.
const POINTS_PER_NOTCH: f64 = 50.0;
const ZOOM_PER_NOTCH: f64 = 1.2;

/// Which pair of world axes a 2D viewport shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis2D {
    /// Looking down: X right, Y up.
    Top,
    /// Y right, Z up.
    Front,
    /// X right, Z up.
    Side,
}

impl Axis2D {
    /// The label Hammer prints in the viewport corner.
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "top (x/y)",
            Self::Front => "front (y/z)",
            Self::Side => "side (x/z)",
        }
    }

    /// Drops the axis this view looks along.
    pub fn project(self, point: DVec3) -> DVec2 {
        match self {
            Self::Top => DVec2::new(point.x, point.y),
            Self::Front => DVec2::new(point.y, point.z),
            Self::Side => DVec2::new(point.x, point.z),
        }
    }
}

/// Pan and zoom state of one 2D viewport.
pub struct View2D {
    pub axis: Axis2D,
    /// World position at the centre of the viewport.
    pub center: DVec2,
    /// Pixels per world unit.
    pub scale: f64,
}

impl View2D {
    pub fn new(axis: Axis2D) -> Self {
        Self {
            axis,
            center: DVec2::ZERO,
            scale: 0.25,
        }
    }

    /// Converts a 2D world position to a screen position inside `rect`.
    pub fn to_screen(&self, rect: Rect, world: DVec2) -> Pos2 {
        let center = rect.center();
        Pos2::new(
            center.x + ((world.x - self.center.x) * self.scale) as f32,
            center.y - ((world.y - self.center.y) * self.scale) as f32,
        )
    }

    /// Converts a screen position inside `rect` to a 2D world position.
    pub fn to_world(&self, rect: Rect, screen: Pos2) -> DVec2 {
        let center = rect.center();
        DVec2::new(
            self.center.x + f64::from(screen.x - center.x) / self.scale,
            self.center.y - f64::from(screen.y - center.y) / self.scale,
        )
    }

    /// Sets the zoom in pixels per world unit, clamped to the supported range.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    }

    /// Centres and zooms the view so the bounds fill most of it.
    pub fn fit(&mut self, bounds: &Bounds, rect: Rect) {
        if bounds.is_empty() || rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        let min = self.axis.project(bounds.min);
        let max = self.axis.project(bounds.max);
        let size = (max - min).max(DVec2::splat(64.0));
        self.center = (min + max) * 0.5;
        let fit = (f64::from(rect.width()) / size.x).min(f64::from(rect.height()) / size.y);
        self.scale = (fit * 0.9).clamp(MIN_SCALE, MAX_SCALE);
    }

    /// Pans with middle-drag or Space+left-drag and zooms around the cursor with the wheel.
    pub fn handle_input(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        let space_held = ui.input(|input| input.key_down(egui::Key::Space));
        let panning = response.dragged_by(PointerButton::Middle)
            || (space_held && response.dragged_by(PointerButton::Primary));
        if panning {
            let delta = response.drag_delta();
            self.center.x -= f64::from(delta.x) / self.scale;
            self.center.y += f64::from(delta.y) / self.scale;
        }

        let scroll = ui.input(|input| input.raw_scroll_delta.y);
        let Some(pointer) = response.hover_pos().filter(|_| scroll != 0.0) else {
            return;
        };
        let anchor = self.to_world(rect, pointer);
        let factor = ZOOM_PER_NOTCH.powf(f64::from(scroll) / POINTS_PER_NOTCH);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let center = rect.center();
        self.center.x = anchor.x - f64::from(pointer.x - center.x) / self.scale;
        self.center.y = anchor.y + f64::from(pointer.y - center.y) / self.scale;
    }

    /// Draws the grid, every wireframe that overlaps the view, and the corner label.
    pub fn draw(&self, canvas: &mut dyn Canvas, rect: Rect, scene: &Scene, grid: f64, active: bool) {
        canvas.fill(rect, Color32::BLACK);
        self.draw_grid(canvas, rect, grid);

        let view_min = self.to_world(rect, rect.left_bottom());
        let view_max = self.to_world(rect, rect.right_top());
        let mut segments = Vec::new();
        for mesh in &scene.meshes {
            let min = self.axis.project(mesh.bounds.min);
            let max = self.axis.project(mesh.bounds.max);
            if max.x < view_min.x || min.x > view_max.x || max.y < view_min.y || min.y > view_max.y {
                continue;
            }
            segments.clear();
            for [a, b] in &mesh.edges {
                let (a, b) = (self.axis.project(*a), self.axis.project(*b));
                if a.distance_squared(b) < 1e-6 {
                    continue;
                }
                segments.push([self.to_screen(rect, a), self.to_screen(rect, b)]);
            }
            canvas.lines(&segments, mesh.color);
        }

        let label_color = if active { Color32::WHITE } else { Color32::GRAY };
        canvas.label(rect.left_top() + egui::vec2(6.0, 4.0), self.axis.label(), label_color);
    }

    fn draw_grid(&self, canvas: &mut dyn Canvas, rect: Rect, grid: f64) {
        let mut spacing = grid;
        while spacing * self.scale < MIN_GRID_PIXELS {
            spacing *= 2.0;
        }
        let min = self.to_world(rect, rect.left_bottom());
        let max = self.to_world(rect, rect.right_top());
        if (max.x - min.x) / spacing + (max.y - min.y) / spacing > MAX_GRID_LINES {
            return;
        }
        for vertical in [true, false] {
            let (low, high) = if vertical { (min.x, max.x) } else { (min.y, max.y) };
            let first = (low / spacing).floor() as i64;
            let last = (high / spacing).ceil() as i64;
            for step in first..=last {
                let value = step as f64 * spacing;
                let (a, b) = if vertical {
                    (DVec2::new(value, min.y), DVec2::new(value, max.y))
                } else {
                    (DVec2::new(min.x, value), DVec2::new(max.x, value))
                };
                canvas.lines(&[[self.to_screen(rect, a), self.to_screen(rect, b)]], grid_color(value));
            }
        }
    }
}

/// Hammer-style grid colours: axis lines, every 1024 units, every 64 units, then the rest.
fn grid_color(value: f64) -> Color32 {
    if value == 0.0 {
        Color32::from_rgb(0, 100, 100)
    } else if value % 1024.0 == 0.0 {
        Color32::from_rgb(100, 46, 0)
    } else if value % 64.0 == 0.0 {
        Color32::from_gray(65)
    } else {
        Color32::from_gray(38)
    }
}
