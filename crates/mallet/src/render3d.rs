//! The textured 3D view, drawn with wgpu into an offscreen image.
//!
//! The same image is shown in the camera viewport and read back for MCP screenshots, so
//! both always match. Faces are shaded by direction like Hammer's textured mode; selected
//! brushes are tinted red with red outlines, and tool previews are drawn as lines on top.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use eframe::egui::Color32;
use eframe::wgpu::{self, util::DeviceExt};
use glam::{DVec3, Mat4, Vec3};
use mallet_content::{parallel_map, Materials};

use crate::scene::{box_edges, Overlay, Remap, Scene, SceneFace, WireMesh, SELECTED_COLOR, TOOL_COLOR};
use crate::view3d::Camera3D;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const NEAR: f32 = 1.0;
const FAR: f32 = 65536.0;
/// Texture size assumed for texture coordinates when a material cannot be found.
const MISSING_TEXTURE_SIZE: f32 = 128.0;
/// How strongly selected faces are tinted red.
const SELECTION_TINT: f32 = 0.4;
/// Direction faces are lit from; faces pointing up are brightest, as in Hammer.
const LIGHT_DIRECTION: DVec3 = DVec3::new(0.3, 0.5, 1.0);
/// Key for the plain white texture used by point entity boxes.
const WHITE_KEY: &str = "";

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceVertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    highlight: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color: [f32; 4],
}

/// A texture uploaded to the GPU, with the full size used for texture coordinates.
struct GpuTexture {
    bind_group: wgpu::BindGroup,
    width: f32,
    height: f32,
}

/// Faces in one vertex buffer, drawn in runs that share a texture.
struct FaceMesh {
    buffer: wgpu::Buffer,
    batches: Vec<(String, Range<u32>)>,
}

struct Target {
    size: [u32; 2],
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

/// Renders the camera viewport with wgpu.
pub struct Renderer3D {
    device: wgpu::Device,
    queue: wgpu::Queue,
    face_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Uploaded textures by material key; `None` when the material could not be loaded.
    textures: HashMap<String, Option<GpuTexture>>,
    missing: GpuTexture,
    static_mesh: Option<(u64, FaceMesh)>,
    target: Option<Target>,
}

impl Renderer3D {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mallet 3d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("render3d.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mallet 3d uniforms"),
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
            label: Some("mallet 3d texture"),
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
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mallet 3d uniforms"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mallet 3d uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mallet 3d sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let face_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mallet 3d faces"),
            bind_group_layouts: &[&uniform_layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mallet 3d lines"),
            bind_group_layouts: &[&uniform_layout],
            push_constant_ranges: &[],
        });
        let face_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32];
        let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let face_pipeline = pipeline(
            device,
            &face_layout,
            &shader,
            ("vs_face", "fs_face"),
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FaceVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &face_attributes,
            },
            wgpu::PrimitiveTopology::TriangleList,
            true,
        );
        let line_pipeline = pipeline(
            device,
            &line_layout,
            &shader,
            ("vs_line", "fs_line"),
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &line_attributes,
            },
            wgpu::PrimitiveTopology::LineList,
            false,
        );

        let upload = |image: &Image, width, height| {
            upload(device, queue, &texture_layout, &sampler, &mip_chain(image), width, height)
        };
        let missing = upload(&checkerboard(), MISSING_TEXTURE_SIZE, MISSING_TEXTURE_SIZE);
        let white = upload(&Image::solid(Color32::WHITE), 1.0, 1.0);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            face_pipeline,
            line_pipeline,
            uniform_buffer,
            uniform_group,
            texture_layout,
            sampler,
            textures: HashMap::from([(WHITE_KEY.to_owned(), Some(white))]),
            missing,
            static_mesh: None,
            target: None,
        }
    }

    /// Forgets every uploaded texture, for example after the game folder changed.
    pub fn clear_textures(&mut self) {
        self.textures.retain(|key, _| key == WHITE_KEY);
        self.static_mesh = None;
    }

    /// Draws the scene from the camera into a `width` × `height` image. Returns the
    /// image's view and whether the image was recreated, so callers holding a reference
    /// to the old one can refresh it.
    pub fn render(
        &mut self,
        scene: &Scene,
        overlay: &Overlay,
        camera: &Camera3D,
        size: [u32; 2],
        materials: Option<&mut Materials>,
    ) -> (&wgpu::TextureView, bool) {
        let size = [size[0].max(1), size[1].max(1)];
        let recreated = self.ensure_target(size);
        self.load_textures(scene, materials);

        let up_to_date = self.static_mesh.as_ref().is_some_and(|(generation, _)| *generation == scene.generation);
        if !up_to_date {
            let unselected = scene.meshes.iter().filter(|mesh| !mesh.selected);
            let mesh = self.face_mesh(unselected, None, 0.0);
            self.static_mesh = Some((scene.generation, mesh));
        }
        let selected: Vec<&WireMesh> = scene.meshes.iter().filter(|mesh| mesh.selected).collect();
        let dynamic = self.face_mesh(selected.iter().copied(), overlay.transform.as_ref(), SELECTION_TINT);
        let lines = self.line_buffer(&selected, overlay);

        let aspect = size[0] as f32 / size[1] as f32;
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&view_projection(camera, aspect).to_cols_array()));

        let target = self.target.as_ref().expect("target was just created");
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mallet 3d") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mallet 3d"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.face_pipeline);
            pass.set_bind_group(0, &self.uniform_group, &[]);
            let meshes = self.static_mesh.iter().map(|(_, mesh)| mesh).chain([&dynamic]);
            for mesh in meshes {
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                for (key, range) in &mesh.batches {
                    pass.set_bind_group(1, &self.texture_for(key).bind_group, &[]);
                    pass.draw(range.clone(), 0..1);
                }
            }
            if let Some((buffer, count)) = &lines {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_bind_group(0, &self.uniform_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        (&self.target.as_ref().expect("target exists").color_view, recreated)
    }

    /// Copies the last rendered image back from the GPU as RGBA bytes, row by row.
    pub fn read_pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let target = self.target.as_ref()?;
        let [width, height] = target.size;
        let row = width * 4;
        let padded_row = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mallet 3d readback"),
            size: u64::from(padded_row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mallet 3d readback") });
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

    fn ensure_target(&mut self, size: [u32; 2]) -> bool {
        if self.target.as_ref().is_some_and(|target| target.size == size) {
            return false;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mallet 3d color"),
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
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mallet 3d depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.target = Some(Target {
            size,
            color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            color,
        });
        true
    }

    /// Uploads the textures of every material in the scene that has not been seen yet.
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
        let keys: Vec<String> = wanted.into_iter().collect();
        let names: Vec<&str> = keys.iter().map(String::as_str).collect();
        let images = match materials {
            Some(materials) => materials.textures(&names),
            None => vec![None; keys.len()],
        };
        let prepared = parallel_map(&images, |image| {
            image.as_ref().map(|image| {
                let pixels = Image {
                    width: image.pixel_width,
                    height: image.pixel_height,
                    rgba: image.rgba.clone(),
                };
                (mip_chain(&pixels), image.width as f32, image.height as f32)
            })
        });
        for (key, prepared) in keys.into_iter().zip(prepared) {
            let texture = prepared.map(|(levels, width, height)| {
                upload(&self.device, &self.queue, &self.texture_layout, &self.sampler, &levels, width, height)
            });
            self.textures.insert(key, texture);
        }
        self.static_mesh = None;
    }

    fn texture_for(&self, key: &str) -> &GpuTexture {
        self.textures.get(key).and_then(Option::as_ref).unwrap_or(&self.missing)
    }

    fn face_mesh<'a>(&self, meshes: impl Iterator<Item = &'a WireMesh>, remap: Option<&Remap>, highlight: f32) -> FaceMesh {
        let mut by_texture: HashMap<String, Vec<FaceVertex>> = HashMap::new();
        for mesh in meshes {
            let tint = if mesh.faces.iter().any(|face| face.material.is_none()) {
                let [r, g, b, _] = mesh.color.to_normalized_gamma_f32();
                [r, g, b]
            } else {
                [1.0, 1.0, 1.0]
            };
            for face in &mesh.faces {
                let key = face.material.as_deref().map_or_else(|| WHITE_KEY.to_owned(), material_key);
                let texture = self.texture_for(&key);
                let vertices = by_texture.entry(key).or_default();
                triangulate(face, texture, tint, highlight, remap, vertices);
            }
        }
        let mut keys: Vec<String> = by_texture.keys().cloned().collect();
        keys.sort();
        let mut all = Vec::new();
        let mut batches = Vec::with_capacity(keys.len());
        for key in keys {
            let vertices = &by_texture[&key];
            let start = all.len() as u32;
            all.extend_from_slice(vertices);
            batches.push((key, start..all.len() as u32));
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mallet 3d faces"),
            contents: if all.is_empty() { &[0u8; 4] } else { bytemuck::cast_slice(&all) },
            usage: wgpu::BufferUsages::VERTEX,
        });
        FaceMesh { buffer, batches }
    }

    fn line_buffer(&self, selected: &[&WireMesh], overlay: &Overlay) -> Option<(wgpu::Buffer, u32)> {
        let mut vertices = Vec::new();
        let mut push = |edges: &[[DVec3; 2]], remap: Option<&Remap>, color: Color32| {
            let color = color.to_normalized_gamma_f32();
            for [a, b] in edges {
                for point in [a, b] {
                    let point = remap.map_or(*point, |remap| remap.apply(*point));
                    vertices.push(LineVertex {
                        position: point.as_vec3().to_array(),
                        color,
                    });
                }
            }
        };
        for mesh in selected {
            push(&mesh.edges, overlay.transform.as_ref(), SELECTED_COLOR);
        }
        if let Some(block) = &overlay.block {
            push(&box_edges(block), None, TOOL_COLOR);
        }
        if vertices.is_empty() {
            return None;
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mallet 3d lines"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Some((buffer, vertices.len() as u32))
    }
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
    width: f32,
    height: f32,
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
    GpuTexture { bind_group, width, height }
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

/// Fans a convex face into triangles with Source texture coordinates:
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
    };
    for index in 1..face.vertices.len().saturating_sub(1) {
        out.push(vertex(face.vertices[0]));
        out.push(vertex(face.vertices[index]));
        out.push(vertex(face.vertices[index + 1]));
    }
}

/// The combined view and projection matrix for the camera, with the camera's horizontal
/// field of view.
fn view_projection(camera: &Camera3D, aspect: f32) -> Mat4 {
    let horizontal = (camera.fov as f32).to_radians();
    let vertical = 2.0 * ((horizontal * 0.5).tan() / aspect).atan();
    let projection = Mat4::perspective_rh(vertical, aspect, NEAR, FAR);
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
    depth_test: bool,
) -> wgpu::RenderPipeline {
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
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: depth_test,
            depth_compare: if depth_test {
                wgpu::CompareFunction::Less
            } else {
                wgpu::CompareFunction::Always
            },
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}
