//! A drawing surface shared by the on-screen viewports and offscreen screenshots, so both
//! are produced by the same drawing code.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke};
use tiny_skia::{Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

/// The few primitives the viewports need.
pub trait Canvas {
    /// Fills a rectangle with a solid colour.
    fn fill(&mut self, rect: Rect, color: Color32);
    /// Draws one-pixel line segments in a single colour.
    fn lines(&mut self, segments: &[[Pos2; 2]], color: Color32);
    /// Draws a viewport label with its top-left corner at `pos`.
    fn label(&mut self, pos: Pos2, text: &str, color: Color32);
}

/// Draws into an egui painter, which clips to the viewport it was created for.
pub struct PainterCanvas<'a>(pub &'a Painter);

impl Canvas for PainterCanvas<'_> {
    fn fill(&mut self, rect: Rect, color: Color32) {
        self.0.rect_filled(rect, 0.0, color);
    }

    fn lines(&mut self, segments: &[[Pos2; 2]], color: Color32) {
        let stroke = Stroke::new(1.0_f32, color);
        for segment in segments {
            self.0.line_segment(*segment, stroke);
        }
    }

    fn label(&mut self, pos: Pos2, text: &str, color: Color32) {
        self.0
            .text(pos, Align2::LEFT_TOP, text, FontId::proportional(13.0), color);
    }
}

/// Draws into an in-memory image with tiny-skia, independent of the window.
///
/// Labels are not drawn, because tiny-skia has no text rendering.
pub struct PixmapCanvas {
    pixmap: Pixmap,
}

impl PixmapCanvas {
    /// Creates a transparent image; `None` when a dimension is zero.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Pixmap::new(width, height).map(|pixmap| Self { pixmap })
    }

    /// Wraps opaque RGBA pixels, such as a GPU render read back to memory.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let size = tiny_skia::IntSize::from_wh(width, height)?;
        Pixmap::from_vec(rgba, size).map(|pixmap| Self { pixmap })
    }

    /// Copies another canvas's image into this one at `(x, y)`, scaled by `scale`.
    pub fn draw_canvas_scaled(&mut self, other: &PixmapCanvas, x: f32, y: f32, scale: f32) {
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, x, y);
        self.pixmap
            .draw_pixmap(0, 0, other.pixmap.as_ref(), &paint, transform, None);
    }

    /// Fills the whole image with one colour.
    pub fn fill_all(&mut self, color: Color32) {
        let [r, g, b, a] = color.to_array();
        self.pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    }

    /// Encodes the image as PNG.
    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        self.pixmap.encode_png().map_err(|error| error.to_string())
    }
}

fn paint(color: Color32) -> Paint<'static> {
    let mut paint = Paint::default();
    let [r, g, b, a] = color.to_array();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

impl Canvas for PixmapCanvas {
    fn fill(&mut self, rect: Rect, color: Color32) {
        if let Some(rect) = tiny_skia::Rect::from_ltrb(rect.min.x, rect.min.y, rect.max.x, rect.max.y) {
            self.pixmap.fill_rect(rect, &paint(color), Transform::identity(), None);
        }
    }

    fn lines(&mut self, segments: &[[Pos2; 2]], color: Color32) {
        let mut builder = PathBuilder::new();
        for [a, b] in segments {
            builder.move_to(a.x, a.y);
            builder.line_to(b.x, b.y);
        }
        let Some(path) = builder.finish() else {
            return;
        };
        let stroke = tiny_skia::Stroke {
            width: 1.0,
            ..Default::default()
        };
        self.pixmap
            .stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
    }

    fn label(&mut self, _pos: Pos2, _text: &str, _color: Color32) {}
}
