//! Editor state shared by the window and the MCP server.
//!
//! The window locks it once per frame; the MCP thread locks it for each tool call. Tool
//! calls therefore never wait for a frame, and keep working while the window is covered
//! or minimized.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use eframe::egui::{Color32, Pos2, Rect, Vec2};
use mallet_vmf::edit::Edit;

use crate::canvas::PixmapCanvas;
use crate::document::Document;
use crate::scene::Scene;
use crate::view2d::{Axis2D, View2D};
use crate::view3d::Camera3D;

pub const MIN_GRID: f64 = 1.0;
pub const MAX_GRID: f64 = 512.0;
const DEFAULT_GRID: f64 = 64.0;
/// Pixels between the four viewports.
pub const VIEWPORT_GAP: f32 = 2.0;
/// Viewport area assumed until the window has laid itself out.
const DEFAULT_AREA: Vec2 = Vec2::new(1600.0, 950.0);
const GAP_COLOR: Color32 = Color32::from_gray(30);

/// Part of the viewport area a screenshot covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotTarget {
    All,
    Camera,
    Top,
    Front,
    Side,
}

impl ShotTarget {
    fn index(self) -> Option<usize> {
        match self {
            Self::All => None,
            Self::Camera => Some(0),
            Self::Top => Some(1),
            Self::Front => Some(2),
            Self::Side => Some(3),
        }
    }
}

/// Everything about the open map and how it is being viewed.
pub struct Editor {
    pub document: Option<Document>,
    pub scene: Scene,
    /// Top, front and side views, in that order.
    pub views: [View2D; 3],
    pub camera: Camera3D,
    pub grid: f64,
    /// Latest status line, shown at the bottom of the window.
    pub message: String,
    /// Camera, top, front and side viewport rectangles from the latest window layout.
    viewport_rects: [Rect; 4],
    refit_on_layout: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            document: None,
            scene: Scene::default(),
            views: [
                View2D::new(Axis2D::Top),
                View2D::new(Axis2D::Front),
                View2D::new(Axis2D::Side),
            ],
            camera: Camera3D::new(),
            grid: DEFAULT_GRID,
            message: "Open a map with File > Open or drop a .vmf onto the window.".to_owned(),
            viewport_rects: quad_rects(Rect::from_min_size(Pos2::ZERO, DEFAULT_AREA), VIEWPORT_GAP),
            refit_on_layout: false,
        }
    }
}

/// Locks shared editor state, recovering it if a previous holder panicked.
pub fn lock(editor: &Mutex<Editor>) -> MutexGuard<'_, Editor> {
    editor.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Editor {
    /// Opens a map and frames it in every viewport.
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        match Document::load(path) {
            Ok(document) => {
                self.scene = Scene::build(&document.vmf);
                self.message = format!("Loaded {}", path.display());
                self.document = Some(document);
                self.fit_2d();
                self.fit_3d();
                self.refit_on_layout = true;
                Ok(())
            }
            Err(error) => {
                self.message = format!("Could not open {}: {error}", path.display());
                Err(self.message.clone())
            }
        }
    }

    /// Saves the open map, first moving it to `path` when one is given.
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<PathBuf, String> {
        let document = self.document.as_mut().ok_or("no map is open")?;
        if let Some(path) = path {
            document.path = path;
        }
        let result = document.save();
        self.message = match &result {
            Ok(()) => format!("Saved {}", document.path.display()),
            Err(error) => format!("Could not save {}: {error}", document.path.display()),
        };
        result.map(|()| document.path.clone()).map_err(|_| self.message.clone())
    }

    /// Applies an edit through the undo history and refreshes the viewports.
    pub fn apply_edit(&mut self, edit: &Edit) -> Result<(), String> {
        let document = self.document.as_mut().ok_or("no map is open")?;
        document
            .history
            .apply(&mut document.vmf, edit)
            .map_err(|error| error.to_string())?;
        self.rebuild_scene();
        Ok(())
    }

    /// Undoes or redoes one step. Returns whether anything changed.
    pub fn step_history(&mut self, redo: bool) -> Result<bool, String> {
        let document = self.document.as_mut().ok_or("no map is open")?;
        let result = if redo {
            document.history.redo(&mut document.vmf)
        } else {
            document.history.undo(&mut document.vmf)
        };
        match result {
            Ok(changed) => {
                if changed {
                    self.rebuild_scene();
                }
                Ok(changed)
            }
            Err(error) => {
                self.message = format!("Could not undo: {error}");
                Err(self.message.clone())
            }
        }
    }

    pub fn rebuild_scene(&mut self) {
        self.scene = self
            .document
            .as_ref()
            .map_or_else(Scene::default, |document| Scene::build(&document.vmf));
    }

    pub fn change_grid(&mut self, factor: f64) {
        self.grid = (self.grid * factor).clamp(MIN_GRID, MAX_GRID);
    }

    /// Fits the whole map into the three 2D views.
    pub fn fit_2d(&mut self) {
        for (view, rect) in self.views.iter_mut().zip(&self.viewport_rects[1..]) {
            view.fit(&self.scene.bounds, *rect);
        }
    }

    /// Moves the 3D camera back until the whole map is in view.
    pub fn fit_3d(&mut self) {
        self.camera.look_at(&self.scene.bounds);
    }

    /// Records the window's viewport layout. A map opened before the first layout is
    /// framed again once the real viewport sizes are known.
    pub fn set_layout(&mut self, rects: [Rect; 4]) {
        self.viewport_rects = rects;
        if self.refit_on_layout {
            self.refit_on_layout = false;
            self.fit_2d();
            self.fit_3d();
        }
    }

    /// Renders viewports offscreen at their on-screen size and encodes them as PNG.
    ///
    /// Uses the same drawing code as the window but does not need it to be visible.
    /// Viewport labels are left out.
    pub fn render_png(&self, target: ShotTarget) -> Result<Vec<u8>, String> {
        if let Some(index) = target.index() {
            return self.render_viewport(index)?.encode_png();
        }
        let rects = self.viewport_rects;
        let area = rects[0].union(rects[3]);
        let mut canvas = canvas_for(area.size())?;
        canvas.fill_all(GAP_COLOR);
        for (index, rect) in rects.iter().enumerate() {
            let offset = rect.min - area.min;
            canvas.draw_canvas(&self.render_viewport(index)?, offset.x.round() as i32, offset.y.round() as i32);
        }
        canvas.encode_png()
    }

    fn render_viewport(&self, index: usize) -> Result<PixmapCanvas, String> {
        let size = self.viewport_rects[index].size();
        let local = Rect::from_min_size(Pos2::ZERO, size);
        let mut canvas = canvas_for(size)?;
        match index {
            0 => self.camera.draw(&mut canvas, local, &self.scene, false),
            _ => self.views[index - 1].draw(&mut canvas, local, &self.scene, self.grid, false),
        }
        Ok(canvas)
    }
}

fn canvas_for(size: Vec2) -> Result<PixmapCanvas, String> {
    PixmapCanvas::new(size.x.round() as u32, size.y.round() as u32)
        .ok_or_else(|| "the viewport has no area".to_owned())
}

/// Splits the area into Hammer's default layout: camera top-left, top view top-right,
/// front view bottom-left and side view bottom-right.
pub fn quad_rects(full: Rect, gap: f32) -> [Rect; 4] {
    let mid = full.center();
    let half = gap * 0.5;
    [
        Rect::from_min_max(full.min, Pos2::new(mid.x - half, mid.y - half)),
        Rect::from_min_max(Pos2::new(mid.x + half, full.min.y), Pos2::new(full.max.x, mid.y - half)),
        Rect::from_min_max(Pos2::new(full.min.x, mid.y + half), Pos2::new(mid.x - half, full.max.y)),
        Rect::from_min_max(Pos2::new(mid.x + half, mid.y + half), full.max),
    ]
}
