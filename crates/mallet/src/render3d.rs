//! The viewports drawn with wgpu into offscreen images: the textured 3D camera view and
//! the three 2D views.
//!
//! The camera image is shown in the window and read back for MCP screenshots, so both
//! always match. Faces are shaded by direction like Hammer's textured mode; selected
//! brushes are tinted red with red outlines, and tool previews are drawn as lines on top.
//! Geometry is cached on the GPU until it changes, so moving the camera or panning a 2D
//! view costs almost nothing on the CPU.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::mpsc;

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use eframe::wgpu::{self, util::DeviceExt};
use glam::{DVec3, Mat4, Vec3, Vec4};
use mallet_content::{parallel_map, AlphaMode, MaterialLoader, Materials};

use crate::scene::{
    box_edges, Overlay, Owner, Remap, Scene, SceneFace, WireMesh, HANDLE_COLOR, SELECTED_COLOR, TOOL_COLOR,
};
use crate::view2d::View2D;
use crate::view3d::Camera3D;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const NEAR: f32 = 1.0;
/// Texture size assumed for texture coordinates when a material cannot be found.
const MISSING_TEXTURE_SIZE: f32 = 128.0;
/// How strongly selected faces are tinted red.
const SELECTION_TINT: f32 = 0.4;
/// Direction faces are lit from; faces pointing up are brightest, as in Hammer.
const LIGHT_DIRECTION: DVec3 = DVec3::new(0.3, 0.5, 1.0);
/// Key for the plain white texture used by point entity boxes.
const WHITE_KEY: &str = "";
/// Side length of the world cubes faces are grouped into for culling.
const CHUNK_SIZE: f64 = 1024.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceVertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    highlight: f32,
    /// Alpha-test reference (0 for none) and blend opacity (0 for opaque).
    alpha: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color: [f32; 4],
}

/// A texture uploaded to the GPU, with the full size used for texture coordinates and the
/// way its material uses alpha.
struct GpuTexture {
    bind_group: wgpu::BindGroup,
    width: f32,
    height: f32,
    alpha: AlphaMode,
}

/// A translucent face, kept on the CPU so it can be sorted back to front every frame.
struct TranslucentFace {
    center: Vec3,
    key: String,
    vertices: Vec<FaceVertex>,
    /// What the face belongs to, so a selected brush's cached copy can be left out while
    /// its tinted copy is drawn.
    owner: Option<Owner>,
}

/// Every face of the map: opaque and alpha-tested faces in chunked GPU buffers, and
/// translucent faces for sorting.
struct StaticFaces {
    generation: u64,
    opaque: FaceMesh,
    translucent: Vec<TranslucentFace>,
}

/// How the camera viewport draws the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraStyle {
    Textured,
    Wireframe,
}

/// How a pipeline uses the depth buffer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// Normal 3D geometry: the nearest surface wins.
    Test,
    /// Drawn over geometry already at the same depth, for the selection tint.
    Overdraw,
    /// Always on top, in a pass that has a depth buffer.
    Ignore,
    /// For passes without a depth buffer.
    None,
    /// Blended translucent faces: tested against the depth buffer but not written to it.
    Translucent,
}

/// Faces in one vertex buffer, split into spatial chunks for culling, each drawn in runs
/// that share a texture.
struct FaceMesh {
    buffer: wgpu::Buffer,
    chunks: Vec<Chunk>,
}

/// The faces of the brushes whose centres fall in one cube of the world.
struct Chunk {
    min: Vec3,
    max: Vec3,
    batches: Vec<(String, Range<u32>)>,
    triangles: usize,
}

/// Lines in one vertex buffer.
struct LineMesh {
    buffer: wgpu::Buffer,
    count: u32,
}

/// An image a viewport is drawn into, with its own camera matrix.
struct Target {
    size: [u32; 2],
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: Option<wgpu::TextureView>,
    uniform_buffer: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
}

/// Draw counts from the latest camera frame, to show what culling saves.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub chunks: usize,
    pub chunks_drawn: usize,
    pub triangles_drawn: usize,
}

/// A texture decoded and mip-mapped on a worker thread, ready to upload: the mip levels,
/// the texture's nominal size and its alpha mode.
type Prepared = (Vec<Image>, (f32, f32), AlphaMode);

/// A batch of textures being loaded in the background.
type Incoming = mpsc::Receiver<Vec<(String, Option<Prepared>)>>;

/// Renders the camera viewport and the three 2D viewports with wgpu.
pub struct Renderer3D {
    device: wgpu::Device,
    queue: wgpu::Queue,
    face_pipeline: wgpu::RenderPipeline,
    overdraw_pipeline: wgpu::RenderPipeline,
    translucent_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    flat_line_pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Uploaded textures by material key; `None` when the material could not be loaded.
    textures: HashMap<String, Option<GpuTexture>>,
    missing: GpuTexture,
    /// Flat grey stand-in drawn while a material's texture is still loading.
    loading_texture: GpuTexture,
    /// Materials being loaded in the background, and the batches on their way.
    loading: HashSet<String>,
    incoming: Vec<Incoming>,
    /// Counts texture uploads, so cached viewport images know when to redraw.
    textures_version: u64,
    /// What each slot's image was last drawn from; an unchanged signature means the image
    /// can be shown again without drawing.
    signatures: [Option<u64>; 4],
    /// Which slots reused their image in their latest draw.
    pub reused: [bool; 4],
    /// Every face, cached until the geometry changes.
    faces: Option<StaticFaces>,
    /// Every edge, cached until the geometry changes; shared by the 2D views.
    edges: Option<(u64, LineMesh)>,
    /// The geometry generation whose materials have been loaded.
    textures_checked: u64,
    /// One image per viewport slot.
    targets: [Option<Target>; 4],
    pub stats: RenderStats,
}

impl Renderer3D {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mallet viewport shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("render3d.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mallet uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mallet texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mallet sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let face_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mallet faces"),
            bind_group_layouts: &[&uniform_layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mallet lines"),
            bind_group_layouts: &[&uniform_layout],
            push_constant_ranges: &[],
        });
        let face_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32, 4 => Float32x2];
        let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let face_buffer = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FaceVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &face_attributes,
        };
        let line_buffer = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attributes,
        };
        let faces = |depth| {
            let topology = wgpu::PrimitiveTopology::TriangleList;
            pipeline(
                device,
                &face_layout,
                &shader,
                ("vs_face", "fs_face"),
                face_buffer.clone(),
                topology,
                depth,
            )
        };
        let lines = |depth| {
            let topology = wgpu::PrimitiveTopology::LineList;
            pipeline(
                device,
                &line_layout,
                &shader,
                ("vs_line", "fs_line"),
                line_buffer.clone(),
                topology,
                depth,
            )
        };
        let face_pipeline = faces(Depth::Test);
        let overdraw_pipeline = faces(Depth::Overdraw);
        let translucent_pipeline = faces(Depth::Translucent);
        let line_pipeline = lines(Depth::Ignore);
        let flat_line_pipeline = lines(Depth::None);

        let upload = |image: &Image, width, height, alpha| {
            upload(
                device,
                queue,
                &texture_layout,
                &sampler,
                &mip_chain(image),
                (width, height),
                alpha,
            )
        };
        let missing = upload(
            &checkerboard(),
            MISSING_TEXTURE_SIZE,
            MISSING_TEXTURE_SIZE,
            AlphaMode::Opaque,
        );
        let white = upload(&Image::solid(Color32::WHITE), 1.0, 1.0, AlphaMode::Opaque);
        let loading_texture = upload(
            &Image::solid(Color32::from_gray(90)),
            MISSING_TEXTURE_SIZE,
            MISSING_TEXTURE_SIZE,
            AlphaMode::Opaque,
        );
        Self {
            device: device.clone(),
            queue: queue.clone(),
            face_pipeline,
            overdraw_pipeline,
            translucent_pipeline,
            line_pipeline,
            flat_line_pipeline,
            uniform_layout,
            texture_layout,
            sampler,
            textures: HashMap::from([(WHITE_KEY.to_owned(), Some(white))]),
            missing,
            loading_texture,
            loading: HashSet::new(),
            incoming: Vec::new(),
            textures_version: 0,
            signatures: [None; 4],
            reused: [false; 4],
            faces: None,
            edges: None,
            textures_checked: 0,
            targets: [None, None, None, None],
            stats: RenderStats::default(),
        }
    }

    /// Forgets every uploaded texture, for example after the game folder changed.
    pub fn clear_textures(&mut self) {
        self.textures.retain(|key, _| key == WHITE_KEY);
        self.loading.clear();
        self.incoming.clear();
        self.faces = None;
        self.textures_checked = 0;
        self.textures_version += 1;
    }

    /// How many materials are still loading in the background.
    pub fn textures_loading(&self) -> usize {
        self.loading.len()
    }

    /// Waits for every background texture load and uploads the results, so a screenshot
    /// shows real textures instead of loading placeholders.
    pub fn finish_loading(&mut self) {
        for batch in std::mem::take(&mut self.incoming) {
            if let Ok(results) = batch.recv() {
                self.upload_results(results);
            }
        }
    }

    /// Uploads whatever background loads have finished, without waiting for the rest.
    fn poll_loading(&mut self) {
        let mut pending = Vec::new();
        for batch in std::mem::take(&mut self.incoming) {
            match batch.try_recv() {
                Ok(results) => self.upload_results(results),
                Err(mpsc::TryRecvError::Empty) => pending.push(batch),
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }
        self.incoming = pending;
        if self.incoming.is_empty() {
            self.loading.clear();
        }
    }

    fn upload_results(&mut self, results: Vec<(String, Option<Prepared>)>) {
        for (key, prepared) in results {
            let texture = prepared.map(|(levels, size, alpha)| {
                upload(
                    &self.device,
                    &self.queue,
                    &self.texture_layout,
                    &self.sampler,
                    &levels,
                    size,
                    alpha,
                )
            });
            self.loading.remove(&key);
            self.textures.insert(key, texture);
        }
        self.faces = None;
        self.textures_version += 1;
    }

    /// Draws the scene from the camera into viewport slot `slot`'s image of `size`
    /// pixels. Returns the image's view and whether the image was recreated, so callers
    /// holding a reference to the old one can refresh it.
    ///
    /// Textured: static geometry comes from a cache split into chunks, and chunks outside
    /// the camera's view are skipped. Faces are one-sided, so brushes look solid from
    /// outside and open from inside, as in Hammer. Translucent faces are drawn after the
    /// rest, sorted back to front. The selection is drawn again on top, tinted, at its
    /// live position. Wireframe: every brush edge in its editor colour.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        slot: usize,
        style: CameraStyle,
        scene: &Scene,
        overlay: &Overlay,
        camera: &Camera3D,
        size: [u32; 2],
        materials: Option<&mut Materials>,
    ) -> (&wgpu::TextureView, bool) {
        let size = [size[0].max(1), size[1].max(1)];
        let recreated = ensure_target(&self.device, &self.uniform_layout, &mut self.targets[slot], size, true);
        self.poll_loading();
        let signature = signature(|hasher| {
            (slot, style == CameraStyle::Textured, size).hash(hasher);
            (
                scene.geometry_generation,
                scene.selection_generation,
                self.textures_version,
            )
                .hash(hasher);
            for value in [
                camera.position.x,
                camera.position.y,
                camera.position.z,
                camera.yaw,
                camera.pitch,
                camera.fov,
                camera.far,
            ] {
                value.to_bits().hash(hasher);
            }
            format!("{overlay:?}").hash(hasher);
        });
        self.reused[slot] = !recreated && self.signatures[slot] == Some(signature);
        if self.reused[slot] {
            return (&self.targets[slot].as_ref().expect("target exists").color_view, false);
        }
        self.signatures[slot] = Some(signature);
        if style == CameraStyle::Textured {
            if self.textures_checked != scene.geometry_generation {
                self.load_textures(scene, materials);
                self.textures_checked = scene.geometry_generation;
            }
            if self
                .faces
                .as_ref()
                .is_none_or(|faces| faces.generation != scene.geometry_generation)
            {
                let (opaque, translucent) = self.face_mesh(scene.meshes.iter(), None, 0.0, true);
                self.faces = Some(StaticFaces {
                    generation: scene.geometry_generation,
                    opaque,
                    translucent,
                });
            }
        } else {
            self.ensure_edges(scene);
        }
        let selected: Vec<&WireMesh> = scene.meshes.iter().filter(|mesh| mesh.selected).collect();
        let (tinted, tinted_translucent) = if style == CameraStyle::Textured {
            let (opaque, translucent) = self.face_mesh(
                selected.iter().copied(),
                overlay.transform.as_ref(),
                SELECTION_TINT,
                false,
            );
            (Some(opaque), translucent)
        } else {
            (None, Vec::new())
        };
        let selected_owners: HashSet<Owner> = selected.iter().filter_map(|mesh| mesh.owner).collect();
        let mut outline = Vec::new();
        for mesh in &selected {
            push_edges(&mut outline, &mesh.edges, overlay.transform.as_ref(), SELECTED_COLOR);
        }
        if let Some(block) = &overlay.block {
            push_edges(&mut outline, &box_edges(block), None, TOOL_COLOR);
        }
        let outline = self.line_mesh(&outline);

        let aspect = size[0] as f32 / size[1] as f32;
        let matrix = view_projection(camera, aspect);
        let planes = frustum_planes(matrix);
        let translucent = match (&self.faces, style) {
            (Some(faces), CameraStyle::Textured) => {
                let unselected = faces
                    .translucent
                    .iter()
                    .filter(|face| face.owner.is_none_or(|owner| !selected_owners.contains(&owner)));
                self.sorted_translucent(unselected.chain(&tinted_translucent), camera, &planes)
            }
            _ => None,
        };
        let target = self.targets[slot].as_ref().expect("target was just created");
        self.queue
            .write_buffer(&target.uniform_buffer, 0, bytemuck::cast_slice(&matrix.to_cols_array()));

        let mut stats = RenderStats::default();
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mallet camera"),
        });
        {
            let mut pass = begin_pass(&mut encoder, target);
            pass.set_bind_group(0, &target.uniform_group, &[]);
            match (style, &self.faces) {
                (CameraStyle::Textured, Some(faces)) => {
                    pass.set_pipeline(&self.face_pipeline);
                    pass.set_vertex_buffer(0, faces.opaque.buffer.slice(..));
                    stats.chunks = faces.opaque.chunks.len();
                    for chunk in faces
                        .opaque
                        .chunks
                        .iter()
                        .filter(|chunk| box_visible(&planes, chunk.min, chunk.max))
                    {
                        stats.chunks_drawn += 1;
                        stats.triangles_drawn += chunk.triangles;
                        for (key, range) in &chunk.batches {
                            pass.set_bind_group(1, &self.texture_for(key).bind_group, &[]);
                            pass.draw(range.clone(), 0..1);
                        }
                    }
                    if let Some(tinted) = &tinted {
                        pass.set_pipeline(&self.overdraw_pipeline);
                        pass.set_vertex_buffer(0, tinted.buffer.slice(..));
                        for (key, range) in tinted.chunks.iter().flat_map(|chunk| &chunk.batches) {
                            pass.set_bind_group(1, &self.texture_for(key).bind_group, &[]);
                            pass.draw(range.clone(), 0..1);
                        }
                    }
                    if let Some((buffer, batches)) = &translucent {
                        pass.set_pipeline(&self.translucent_pipeline);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        for (key, range) in batches {
                            stats.triangles_drawn += range.len() / 3;
                            pass.set_bind_group(1, &self.texture_for(key).bind_group, &[]);
                            pass.draw(range.clone(), 0..1);
                        }
                    }
                }
                (CameraStyle::Wireframe, _) => {
                    if let Some((_, edges)) = &self.edges {
                        pass.set_pipeline(&self.line_pipeline);
                        pass.set_vertex_buffer(0, edges.buffer.slice(..));
                        pass.draw(0..edges.count, 0..1);
                    }
                }
                (CameraStyle::Textured, None) => {}
            }
            if let Some(lines) = &outline {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_vertex_buffer(0, lines.buffer.slice(..));
                pass.draw(0..lines.count, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        if style == CameraStyle::Textured {
            self.stats = stats;
        }
        (
            &self.targets[slot].as_ref().expect("target exists").color_view,
            recreated,
        )
    }

    /// Builds this frame's buffer of translucent faces in view, farthest first, with runs
    /// of the same texture merged into one draw.
    #[allow(clippy::type_complexity)]
    fn sorted_translucent<'a>(
        &self,
        faces: impl Iterator<Item = &'a TranslucentFace>,
        camera: &Camera3D,
        planes: &[Vec4; 6],
    ) -> Option<(wgpu::Buffer, Vec<(String, Range<u32>)>)> {
        let eye = camera.position.as_vec3();
        let mut visible: Vec<&TranslucentFace> = faces
            .filter(|face| box_visible(planes, face.center, face.center))
            .collect();
        if visible.is_empty() {
            return None;
        }
        visible.sort_by(|a, b| {
            b.center
                .distance_squared(eye)
                .total_cmp(&a.center.distance_squared(eye))
        });
        let mut vertices = Vec::new();
        let mut batches: Vec<(String, Range<u32>)> = Vec::new();
        for face in visible {
            let start = vertices.len() as u32;
            vertices.extend_from_slice(&face.vertices);
            let end = vertices.len() as u32;
            match batches.last_mut() {
                Some((key, range)) if *key == face.key => range.end = end,
                _ => batches.push((face.key.clone(), start..end)),
            }
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mallet translucent faces"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Some((buffer, batches))
    }

    /// Rebuilds the cached buffer of every brush edge when the geometry changed.
    fn ensure_edges(&mut self, scene: &Scene) {
        if self
            .edges
            .as_ref()
            .is_none_or(|(generation, _)| *generation != scene.geometry_generation)
        {
            let mut vertices = Vec::new();
            for mesh in &scene.meshes {
                push_edges(&mut vertices, &mesh.edges, None, mesh.color);
            }
            self.edges = self.line_mesh(&vertices).map(|mesh| (scene.geometry_generation, mesh));
        }
    }

    /// Draws one 2D view into viewport slot `slot`'s image of `pixels` size; `points` is the viewport's size
    /// in layout points, which the view's zoom is measured in. Returns the image's view
    /// and whether it was recreated.
    ///
    /// Brush edges come from a cache shared by all three views, so panning and zooming
    /// only change a matrix. Handles' squares, the marquee and the label are left for the
    /// caller to draw on top.
    #[allow(clippy::too_many_arguments)]
    pub fn render_2d(
        &mut self,
        slot: usize,
        view: &View2D,
        points: Vec2,
        pixels: [u32; 2],
        scene: &Scene,
        overlay: &Overlay,
        grid: f64,
    ) -> (&wgpu::TextureView, bool) {
        let pixels = [pixels[0].max(1), pixels[1].max(1)];
        let recreated = ensure_target(
            &self.device,
            &self.uniform_layout,
            &mut self.targets[slot],
            pixels,
            false,
        );
        let signature = signature(|hasher| {
            (slot, view.axis.axes(), pixels).hash(hasher);
            (scene.geometry_generation, scene.selection_generation).hash(hasher);
            for value in [
                view.center.x,
                view.center.y,
                view.scale,
                grid,
                f64::from(points.x),
                f64::from(points.y),
            ] {
                value.to_bits().hash(hasher);
            }
            format!("{overlay:?}").hash(hasher);
        });
        self.reused[slot] = !recreated && self.signatures[slot] == Some(signature);
        if self.reused[slot] {
            return (&self.targets[slot].as_ref().expect("target exists").color_view, false);
        }
        self.signatures[slot] = Some(signature);
        self.ensure_edges(scene);

        let local = Rect::from_min_size(Pos2::ZERO, points);
        let mut under = Vec::new();
        for ([a, b], color) in view.grid_lines(local, grid) {
            push_edges(&mut under, &[[view.axis.lift(a), view.axis.lift(b)]], None, color);
        }
        let mut over = Vec::new();
        if let Some(handles) = &overlay.handles {
            for [a, b] in view.handle_outline(handles) {
                push_edges(&mut over, &[[view.axis.lift(a), view.axis.lift(b)]], None, HANDLE_COLOR);
            }
        }
        for ([a, b], color) in view.centre_crosses(local, scene, overlay) {
            push_edges(&mut over, &[[view.axis.lift(a), view.axis.lift(b)]], None, color);
        }
        for mesh in scene.meshes.iter().filter(|mesh| mesh.selected) {
            push_edges(&mut over, &mesh.edges, overlay.transform.as_ref(), SELECTED_COLOR);
        }
        if let Some(block) = &overlay.block {
            push_edges(&mut over, &box_edges(block), None, TOOL_COLOR);
        }
        let under = self.line_mesh(&under);
        let over = self.line_mesh(&over);

        let target = self.targets[slot].as_ref().expect("target was just created");
        let matrix = orthographic(view, points);
        self.queue
            .write_buffer(&target.uniform_buffer, 0, bytemuck::cast_slice(&matrix.to_cols_array()));
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mallet 2d"),
        });
        {
            let mut pass = begin_pass(&mut encoder, target);
            pass.set_pipeline(&self.flat_line_pipeline);
            pass.set_bind_group(0, &target.uniform_group, &[]);
            let static_edges = self.edges.as_ref().map(|(_, mesh)| mesh);
            for lines in [under.as_ref(), static_edges, over.as_ref()].into_iter().flatten() {
                pass.set_vertex_buffer(0, lines.buffer.slice(..));
                pass.draw(0..lines.count, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        (
            &self.targets[slot].as_ref().expect("target exists").color_view,
            recreated,
        )
    }

    /// Copies the last rendered image of a viewport slot back from the GPU as RGBA bytes,
    /// row by row.
    pub fn read_pixels(&self, slot: usize) -> Option<(u32, u32, Vec<u8>)> {
        let target = self.targets.get(slot)?.as_ref()?;
        let [width, height] = target.size;
        let row = width * 4;
        let padded_row = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mallet readback"),
            size: u64::from(padded_row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mallet readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::Maintain::Wait);
        receiver.recv().ok()?.ok()?;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((row * height) as usize);
        for line in mapped.chunks(padded_row as usize) {
            pixels.extend_from_slice(&line[..row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Some((width, height, pixels))
    }

    /// Starts loading, on a background thread, every material in the scene that has not
    /// been seen yet. Faces show a grey stand-in until their texture arrives.
    fn load_textures(&mut self, scene: &Scene, materials: Option<&mut Materials>) {
        let wanted: HashSet<String> = scene
            .meshes
            .iter()
            .flat_map(|mesh| &mesh.faces)
            .filter_map(|face| face.material.as_deref())
            .map(material_key)
            .filter(|key| !self.textures.contains_key(key))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let keys: Vec<String> = wanted.into_iter().filter(|key| !self.loading.contains(key)).collect();
        if keys.is_empty() {
            return;
        }
        let Some(materials) = materials else {
            for key in keys {
                self.textures.insert(key, None);
            }
            return;
        };
        let loader: MaterialLoader = materials.loader();
        self.loading.extend(keys.iter().cloned());
        let (sender, receiver) = mpsc::channel();
        self.incoming.push(receiver);
        std::thread::Builder::new()
            .name("mallet textures".to_owned())
            .spawn(move || {
                let prepared = parallel_map(&keys, |key| {
                    let material = loader.load(key);
                    let prepared = material.texture.as_ref().map(|image| {
                        let pixels = Image {
                            width: image.pixel_width,
                            height: image.pixel_height,
                            rgba: image.rgba.clone(),
                        };
                        (
                            mip_chain(&pixels),
                            (image.width as f32, image.height as f32),
                            material.alpha,
                        )
                    });
                    (key.clone(), prepared)
                });
                let _ = sender.send(prepared);
            })
            .expect("texture loading thread could not start");
    }

    fn texture_for(&self, key: &str) -> &GpuTexture {
        if self.loading.contains(key) {
            return &self.loading_texture;
        }
        self.textures.get(key).and_then(Option::as_ref).unwrap_or(&self.missing)
    }

    /// Builds a vertex buffer for the faces of `meshes`. With `chunked`, brushes are
    /// grouped by which world cube their centre is in, so whole chunks can be culled.
    /// Translucent faces are always returned separately, for sorting.
    fn face_mesh<'a>(
        &self,
        meshes: impl Iterator<Item = &'a WireMesh>,
        remap: Option<&Remap>,
        highlight: f32,
        chunked: bool,
    ) -> (FaceMesh, Vec<TranslucentFace>) {
        let mut translucent = Vec::new();
        type ChunkFaces = (Vec3, Vec3, HashMap<String, Vec<FaceVertex>>);
        let mut chunks: HashMap<[i32; 3], ChunkFaces> = HashMap::new();
        for mesh in meshes {
            let bounds = mesh.mapped_bounds(remap);
            let cell = if chunked {
                (bounds.center() / CHUNK_SIZE).floor().as_ivec3().to_array()
            } else {
                [0; 3]
            };
            let chunk = chunks.entry(cell).or_insert_with(|| {
                (
                    Vec3::splat(f32::INFINITY),
                    Vec3::splat(f32::NEG_INFINITY),
                    HashMap::new(),
                )
            });
            chunk.0 = chunk.0.min(bounds.min.as_vec3());
            chunk.1 = chunk.1.max(bounds.max.as_vec3());
            let tint = if mesh.faces.iter().any(|face| face.material.is_none()) {
                let [r, g, b, _] = egui::Rgba::from(mesh.color).to_array();
                [r, g, b]
            } else {
                [1.0, 1.0, 1.0]
            };
            for face in &mesh.faces {
                let key = face
                    .material
                    .as_deref()
                    .map_or_else(|| WHITE_KEY.to_owned(), material_key);
                let texture = self.texture_for(&key);
                if matches!(texture.alpha, AlphaMode::Blend(_)) {
                    let mut vertices = Vec::new();
                    triangulate(face, texture, tint, highlight, remap, &mut vertices);
                    let center = face.vertices.iter().sum::<DVec3>() / face.vertices.len().max(1) as f64;
                    translucent.push(TranslucentFace {
                        center: center.as_vec3(),
                        owner: mesh.owner,
                        key,
                        vertices,
                    });
                } else {
                    triangulate(face, texture, tint, highlight, remap, chunk.2.entry(key).or_default());
                }
            }
        }

        let mut all = Vec::new();
        let mut built = Vec::with_capacity(chunks.len());
        for (_, (min, max, by_texture)) in chunks {
            let mut keys: Vec<String> = by_texture.keys().cloned().collect();
            keys.sort();
            let mut batches = Vec::with_capacity(keys.len());
            let first = all.len();
            for key in keys {
                let start = all.len() as u32;
                all.extend_from_slice(&by_texture[&key]);
                batches.push((key, start..all.len() as u32));
            }
            built.push(Chunk {
                min,
                max,
                batches,
                triangles: (all.len() - first) / 3,
            });
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mallet faces"),
            contents: if all.is_empty() {
                &[0u8; 4]
            } else {
                bytemuck::cast_slice(&all)
            },
            usage: wgpu::BufferUsages::VERTEX,
        });
        (FaceMesh { buffer, chunks: built }, translucent)
    }

    fn line_mesh(&self, vertices: &[LineVertex]) -> Option<LineMesh> {
        if vertices.is_empty() {
            return None;
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mallet lines"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Some(LineMesh {
            buffer,
            count: vertices.len() as u32,
        })
    }
}

/// Hashes whatever `feed` writes, to fingerprint what a viewport image was drawn from.
fn signature(feed: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut hasher = DefaultHasher::new();
    feed(&mut hasher);
    hasher.finish()
}

/// Adds line-list vertices for edges, optionally remapped, in one colour.
fn push_edges(out: &mut Vec<LineVertex>, edges: &[[DVec3; 2]], remap: Option<&Remap>, color: Color32) {
    let color = egui::Rgba::from(color).to_array();
    for [a, b] in edges {
        for point in [a, b] {
            let point = remap.map_or(*point, |remap| remap.apply(*point));
            out.push(LineVertex {
                position: point.as_vec3().to_array(),
                color,
            });
        }
    }
}

/// Creates a viewport's image, or recreates it when its size changed or the slot switched
/// between 3D (with a depth buffer) and 2D. Returns whether it was recreated.
fn ensure_target(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    slot: &mut Option<Target>,
    size: [u32; 2],
    with_depth: bool,
) -> bool {
    if slot
        .as_ref()
        .is_some_and(|target| target.size == size && target.depth_view.is_some() == with_depth)
    {
        return false;
    }
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mallet viewport color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_view = with_depth.then(|| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("mallet viewport depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    });
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mallet viewport matrix"),
        size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mallet viewport matrix"),
        layout: uniform_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });
    *slot = Some(Target {
        size,
        color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
        color,
        depth_view,
        uniform_buffer,
        uniform_group,
    });
    true
}

fn begin_pass<'a>(encoder: &'a mut wgpu::CommandEncoder, target: &'a Target) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("mallet viewport"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &target.color_view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: target
            .depth_view
            .as_ref()
            .map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

/// The six clipping planes of a view-projection matrix, each as `normal.xyz, distance`,
/// with normals pointing into the visible volume. wgpu's depth runs from 0 to 1.
fn frustum_planes(matrix: Mat4) -> [Vec4; 6] {
    let row = |index| matrix.row(index);
    [
        row(3) + row(0),
        row(3) - row(0),
        row(3) + row(1),
        row(3) - row(1),
        row(2),
        row(3) - row(2),
    ]
}

/// Whether any part of a box can be inside the frustum.
fn box_visible(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|plane| {
        let farthest = Vec3::select(plane.truncate().cmpge(Vec3::ZERO), max, min);
        plane.truncate().dot(farthest) + plane.w >= 0.0
    })
}

/// Maps a 2D view's visible area onto clip space: its two axes become x and y, and the
/// depth axis is ignored.
fn orthographic(view: &View2D, points: Vec2) -> Mat4 {
    let [horizontal, vertical, _] = view.axis.axes();
    let scale_x = (2.0 * view.scale / f64::from(points.x.max(1.0))) as f32;
    let scale_y = (2.0 * view.scale / f64::from(points.y.max(1.0))) as f32;
    let mut columns = [[0.0f32; 4]; 4];
    columns[horizontal][0] = scale_x;
    columns[vertical][1] = scale_y;
    columns[3] = [
        -(view.center.x as f32) * scale_x,
        -(view.center.y as f32) * scale_y,
        0.5,
        1.0,
    ];
    Mat4::from_cols_array_2d(&columns)
}

/// Plain RGBA pixels.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Image {
    fn solid(color: Color32) -> Self {
        Self {
            width: 1,
            height: 1,
            rgba: color.to_array().to_vec(),
        }
    }
}

/// Uploads a texture's mip levels, largest first. `width` and `height` are the texture's
/// nominal size, used for texture coordinates.
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    levels: &[Image],
    (width, height): (f32, f32),
    alpha: AlphaMode,
) -> GpuTexture {
    let image = &levels[0];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mallet material"),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, mip) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mip.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mip.width * 4),
                rows_per_image: Some(mip.height),
            },
            wgpu::Extent3d {
                width: mip.width,
                height: mip.height,
                depth_or_array_layers: 1,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mallet material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    GpuTexture {
        bind_group,
        width,
        height,
        alpha,
    }
}

/// Hammer's pink and black checkerboard for missing textures.
fn checkerboard() -> Image {
    const SIDE: u32 = 64;
    const CELL: u32 = 16;
    let rgba = (0..SIDE * SIDE)
        .flat_map(|index| {
            let (x, y) = (index % SIDE, index / SIDE);
            if (x / CELL + y / CELL).is_multiple_of(2) {
                [255, 0, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        })
        .collect();
    Image {
        width: SIDE,
        height: SIDE,
        rgba,
    }
}

/// Halves the image repeatedly with a 2×2 box filter until it is one pixel wide and high.
fn mip_chain(image: &Image) -> Vec<Image> {
    let mut levels = vec![Image {
        width: image.width,
        height: image.height,
        rgba: image.rgba.clone(),
    }];
    while let Some(last) = levels.last() {
        if last.width == 1 && last.height == 1 {
            break;
        }
        let (width, height) = ((last.width / 2).max(1), (last.height / 2).max(1));
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                for channel in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(last.width - 1);
                        let sy = (y * 2 + dy).min(last.height - 1);
                        sum += u32::from(last.rgba[((sy * last.width + sx) * 4 + channel) as usize]);
                    }
                    rgba.push((sum / 4) as u8);
                }
            }
        }
        levels.push(Image { width, height, rgba });
    }
    levels
}

/// Fans a convex face into triangles wound counter-clockwise as seen from outside, so
/// back-face culling hides faces seen from behind, as in Hammer. Texture coordinates follow
/// Source:
/// `u = (point · axis / scale + offset) / texture width`, and likewise for `v`.
fn triangulate(
    face: &SceneFace,
    texture: &GpuTexture,
    tint: [f32; 3],
    highlight: f32,
    remap: Option<&Remap>,
    out: &mut Vec<FaceVertex>,
) {
    let shade = (0.6 + 0.4 * face.normal.dot(LIGHT_DIRECTION.normalize())).clamp(0.35, 1.0) as f32;
    let color = [tint[0] * shade, tint[1] * shade, tint[2] * shade, 1.0];
    let coordinate = |point: DVec3, axis: &mallet_vmf::TexAxis, size: f32| {
        let scale = if axis.scale == 0.0 { 1.0 } else { axis.scale };
        ((point.dot(axis.axis) / scale + axis.offset) as f32) / size
    };
    let vertex = |point: DVec3| FaceVertex {
        position: remap.map_or(point, |remap| remap.apply(point)).as_vec3().to_array(),
        uv: [
            coordinate(point, &face.uaxis, texture.width),
            coordinate(point, &face.vaxis, texture.height),
        ],
        color,
        highlight,
        alpha: match texture.alpha {
            AlphaMode::Opaque => [0.0, 0.0],
            AlphaMode::Test(reference) => [reference, 0.0],
            AlphaMode::Blend(opacity) => [0.0, opacity.max(f32::EPSILON)],
        },
    };
    let points = &face.vertices;
    if points.len() < 3 {
        return;
    }
    let counter_clockwise = (points[1] - points[0]).cross(points[2] - points[0]).dot(face.normal) >= 0.0;
    for index in 1..points.len() - 1 {
        let (second, third) = if counter_clockwise {
            (index, index + 1)
        } else {
            (index + 1, index)
        };
        out.push(vertex(points[0]));
        out.push(vertex(points[second]));
        out.push(vertex(points[third]));
    }
}

/// The combined view and projection matrix for the camera, with the camera's horizontal
/// field of view. Nothing beyond the camera's draw distance is drawn, and the frustum's
/// far plane makes culling skip it too.
fn view_projection(camera: &Camera3D, aspect: f32) -> Mat4 {
    let far = (camera.far as f32).max(NEAR * 2.0);
    let horizontal = (camera.fov as f32).to_radians();
    let vertical = 2.0 * ((horizontal * 0.5).tan() / aspect).atan();
    let projection = Mat4::perspective_rh(vertical, aspect, NEAR, far);
    let view = Mat4::look_to_rh(camera.position.as_vec3(), camera.forward().as_vec3(), Vec3::Z);
    projection * view
}

fn material_key(material: &str) -> String {
    material.replace('\\', "/").to_ascii_lowercase()
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    (vertex_entry, fragment_entry): (&str, &str),
    buffer: wgpu::VertexBufferLayout<'_>,
    topology: wgpu::PrimitiveTopology,
    depth: Depth,
) -> wgpu::RenderPipeline {
    let depth_stencil = match depth {
        Depth::None => None,
        Depth::Test | Depth::Overdraw | Depth::Ignore | Depth::Translucent => Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: matches!(depth, Depth::Test | Depth::Overdraw),
            depth_compare: match depth {
                Depth::Test | Depth::Translucent => wgpu::CompareFunction::Less,
                Depth::Overdraw => wgpu::CompareFunction::LessEqual,
                _ => wgpu::CompareFunction::Always,
            },
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(vertex_entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            buffers: &[buffer],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: (topology == wgpu::PrimitiveTopology::TriangleList).then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: (depth == Depth::Translucent).then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}
