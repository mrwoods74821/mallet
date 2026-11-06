//! Editor state shared by the window and the MCP server.
//!
//! The window locks it once per frame; the MCP thread locks it for each tool call. Tool
//! calls therefore never wait for a frame, and keep working while the window is covered
//! or minimized.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use eframe::egui::{Color32, Pos2, Rect, Vec2};
use glam::{DVec2, DVec3};
use mallet_vmf::build::box_solid;
use mallet_vmf::edit::Edit;
use mallet_vmf::geometry::Bounds;
use mallet_vmf::{format_vec3, Entity, IdAllocator, Solid, Vmf};

use crate::canvas::PixmapCanvas;
use crate::document::Document;
use crate::scene::{Overlay, Owner, Scene};
use crate::view2d::{Axis2D, View2D};
use crate::view3d::Camera3D;

/// Material given to new brushes until a texture browser exists.
pub const DEFAULT_MATERIAL: &str = "DEV/DEV_MEASUREGENERIC01B";
/// Depth range used for the first block before any block has been created.
const DEFAULT_BLOCK: Bounds = Bounds {
    min: DVec3::ZERO,
    max: DVec3::splat(64.0),
};

/// The active editing tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Hammer's Selection tool (Shift+S).
    Select,
    /// Hammer's Block tool (Shift+B).
    Block,
}

/// How a selection change combines with what is already selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Remove,
    Toggle,
}

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
    pub tool: Tool,
    /// Selected world brushes and entities.
    pub selection: BTreeSet<Owner>,
    /// Tool previews drawn over the map.
    pub overlay: Overlay,
    /// Material applied to new brushes.
    pub material: String,
    /// The last block created; new blocks reuse its depth.
    last_block: Bounds,
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
            tool: Tool::Select,
            selection: BTreeSet::new(),
            overlay: Overlay::default(),
            material: DEFAULT_MATERIAL.to_owned(),
            last_block: DEFAULT_BLOCK,
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
                self.set_document(document);
                self.message = format!("Loaded {}", path.display());
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

    /// Replaces the open map with a new, empty, unsaved one, viewed from the origin.
    pub fn new_map(&mut self) {
        self.set_document(Document::untitled());
        self.views = [
            View2D::new(Axis2D::Top),
            View2D::new(Axis2D::Front),
            View2D::new(Axis2D::Side),
        ];
        self.camera = Camera3D::new();
        self.refit_on_layout = false;
        self.message = "New map".to_owned();
    }

    fn set_document(&mut self, document: Document) {
        self.scene = Scene::build(&document.vmf);
        self.document = Some(document);
        self.selection.clear();
        self.overlay = Overlay::default();
    }

    /// Saves the open map, first moving it to `path` when one is given. A map that has
    /// never been saved needs a path.
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<PathBuf, String> {
        let document = self.document.as_mut().ok_or("no map is open")?;
        if let Some(path) = path {
            document.path = Some(path);
        }
        let result = document.save();
        self.message = match &result {
            Ok(()) => format!("Saved {}", document.display_path()),
            Err(error) => format!("Could not save {}: {error}", document.display_path()),
        };
        match (result, &document.path) {
            (Ok(()), Some(path)) => Ok(path.clone()),
            _ => Err(self.message.clone()),
        }
    }

    /// Whether the open map has never been saved, so saving needs a path.
    pub fn is_untitled(&self) -> bool {
        self.document.as_ref().is_some_and(|document| document.path.is_none())
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

    /// Rebuilds the wireframes after the map changed, dropping selected items that no
    /// longer exist.
    pub fn rebuild_scene(&mut self) {
        self.scene = self
            .document
            .as_ref()
            .map_or_else(Scene::default, |document| Scene::build(&document.vmf));
        let scene = &self.scene;
        self.selection.retain(|owner| scene.contains(*owner));
        self.scene.mark_selected(&self.selection);
    }

    /// Changes the selection. Owners that are not in the map are ignored.
    pub fn select(&mut self, owners: impl IntoIterator<Item = Owner>, mode: SelectMode) {
        let owners: Vec<Owner> = owners
            .into_iter()
            .filter(|owner| self.scene.contains(*owner))
            .collect();
        match mode {
            SelectMode::Replace => self.selection = owners.into_iter().collect(),
            SelectMode::Add => self.selection.extend(owners),
            SelectMode::Remove => {
                for owner in owners {
                    self.selection.remove(&owner);
                }
            }
            SelectMode::Toggle => {
                for owner in owners {
                    if !self.selection.remove(&owner) {
                        self.selection.insert(owner);
                    }
                }
            }
        }
        self.scene.mark_selected(&self.selection);
    }

    pub fn clear_selection(&mut self) {
        self.select([], SelectMode::Replace);
    }

    pub fn select_all(&mut self) {
        let all = self.scene.owners();
        self.select(all, SelectMode::Replace);
    }

    /// A short description of the selection for the status bar, such as
    /// `2 objects selected, 128w 64l 32h`.
    pub fn selection_summary(&self) -> Option<String> {
        if self.selection.is_empty() {
            return None;
        }
        let size = self.scene.selection_bounds().size();
        let count = self.selection.len();
        let noun = if count == 1 { "object" } else { "objects" };
        Some(format!("{count} {noun} selected, {}w {}l {}h", size.x, size.y, size.z))
    }

    /// Moves the selection, with texture lock. Entity origins move with it.
    pub fn move_selection(&mut self, offset: DVec3) -> Result<(), String> {
        let vmf = &self.document.as_ref().ok_or("no map is open")?.vmf;
        if self.selection.is_empty() {
            return Err("nothing is selected".to_owned());
        }
        let mut solids = Vec::new();
        let mut origin_edits = Vec::new();
        for owner in &self.selection {
            match *owner {
                Owner::Solid(id) => solids.push(id),
                Owner::Entity(id) => {
                    let entity = find_entity(vmf, id)?;
                    solids.extend(entity.solids().filter_map(Solid::id));
                    if let Some(origin) = entity.origin() {
                        origin_edits.push(Edit::SetKey {
                            entity: id,
                            key: "origin".to_owned(),
                            value: format_vec3(origin + offset),
                        });
                    }
                }
            }
        }
        let mut edits = vec![Edit::Translate { solids, offset }];
        edits.extend(origin_edits);
        self.apply_edit(&Edit::Batch(edits))?;
        self.message = format!("Moved selection by {}", format_vec3(offset));
        Ok(())
    }

    /// Copies the selection with fresh ids, moves the copies by `offset` and selects them.
    pub fn clone_selection(&mut self, offset: DVec3) -> Result<(), String> {
        let vmf = &self.document.as_ref().ok_or("no map is open")?.vmf;
        if self.selection.is_empty() {
            return Err("nothing is selected".to_owned());
        }
        let world = vmf.world().ok_or("the map has no world")?;
        let world_id = world.id().ok_or("the world has no id")?;
        let mut ids = IdAllocator::for_map(vmf);
        let mut edits = Vec::new();
        let mut copies = Vec::new();
        for owner in &self.selection {
            match *owner {
                Owner::Solid(id) => {
                    let mut solid = world
                        .solids()
                        .find(|solid| solid.id() == Some(id))
                        .ok_or_else(|| format!("no world brush has id {id}"))?
                        .clone();
                    solid.renumber(&mut ids);
                    solid.translate(offset);
                    copies.extend(solid.id().map(Owner::Solid));
                    edits.push(Edit::AddSolid {
                        entity: world_id,
                        index: None,
                        solid,
                    });
                }
                Owner::Entity(id) => {
                    let mut entity = find_entity(vmf, id)?.clone();
                    entity.renumber(&mut ids);
                    entity.translate(offset);
                    copies.extend(entity.id().map(Owner::Entity));
                    edits.push(Edit::AddEntity { index: None, entity });
                }
            }
        }
        self.apply_edit(&Edit::Batch(edits))?;
        self.message = format!("Cloned {} objects", copies.len());
        self.select(copies, SelectMode::Replace);
        Ok(())
    }

    /// Deletes the selected brushes and entities. Returns how many were removed.
    pub fn delete_selection(&mut self) -> Result<usize, String> {
        if self.selection.is_empty() {
            return Err("nothing is selected".to_owned());
        }
        let edits: Vec<Edit> = self
            .selection
            .iter()
            .map(|owner| match *owner {
                Owner::Solid(solid) => Edit::RemoveSolid { solid },
                Owner::Entity(entity) => Edit::RemoveEntity { entity },
            })
            .collect();
        let count = edits.len();
        self.apply_edit(&Edit::Batch(edits))?;
        self.message = format!("Deleted {count} objects");
        Ok(count)
    }

    /// Creates a box brush in the world with the current material and selects it.
    /// Returns the new brush's id.
    pub fn create_block(&mut self, min: DVec3, max: DVec3) -> Result<u32, String> {
        let vmf = &self.document.as_ref().ok_or("no map is open")?.vmf;
        let world_id = vmf.world().and_then(Entity::id).ok_or("the map has no world")?;
        let mut ids = IdAllocator::for_map(vmf);
        let solid = box_solid(min, max, &self.material, &mut ids).ok_or("the block has no volume")?;
        let id = solid.id().ok_or("the new block has no id")?;
        self.apply_edit(&Edit::AddSolid {
            entity: world_id,
            index: None,
            solid,
        })?;
        self.last_block = Bounds {
            min: min.min(max),
            max: min.max(max),
        };
        self.overlay.block = None;
        self.select([Owner::Solid(id)], SelectMode::Replace);
        self.message = format!("Created block {id}");
        Ok(id)
    }

    /// Creates the block tool's pending box.
    pub fn commit_block(&mut self) -> Result<u32, String> {
        let block = self.overlay.block.ok_or("draw a block in a 2D view first")?;
        self.create_block(block.min, block.max)
    }

    /// The box a block-tool drag between two snapped 2D corners describes, taking its
    /// depth from the last block created. `None` when it has no area.
    pub fn block_from_drag(&self, axis: Axis2D, a: DVec2, b: DVec2) -> Option<Bounds> {
        let [horizontal, vertical, _] = axis.axes();
        let mut min = self.last_block.min;
        let mut max = self.last_block.max;
        min[horizontal] = a.x.min(b.x);
        max[horizontal] = a.x.max(b.x);
        min[vertical] = a.y.min(b.y);
        max[vertical] = a.y.max(b.y);
        let bounds = Bounds { min, max };
        (bounds.size().min_element() > 0.0).then_some(bounds)
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
            0 => self.camera.draw(&mut canvas, local, &self.scene, false, &self.overlay),
            _ => self.views[index - 1].draw(&mut canvas, local, &self.scene, self.grid, false, &self.overlay),
        }
        Ok(canvas)
    }
}

fn find_entity(vmf: &Vmf, id: u32) -> Result<&Entity, String> {
    vmf.all_entities()
        .find(|entity| entity.id() == Some(id))
        .ok_or_else(|| format!("no entity has id {id}"))
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
