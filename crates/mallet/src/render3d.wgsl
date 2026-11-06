// Shaders for the textured 3D view.

struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var face_texture: texture_2d<f32>;
@group(1) @binding(1) var face_sampler: sampler;

struct FaceIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) highlight: f32,
    @location(4) alpha: vec2<f32>,
};

struct FaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) highlight: f32,
    @location(3) alpha: vec2<f32>,
};

@vertex
fn vs_face(input: FaceIn) -> FaceOut {
    var out: FaceOut;
    out.clip = uniforms.view_proj * vec4<f32>(input.position, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    out.highlight = input.highlight;
    out.alpha = input.alpha;
    return out;
}

// alpha.x is the alpha-test reference (0 for none); alpha.y the blend opacity (0 for opaque).
@fragment
fn fs_face(input: FaceOut) -> @location(0) vec4<f32> {
    let base = textureSample(face_texture, face_sampler, input.uv) * input.color;
    if (input.alpha.x > 0.0 && base.a < input.alpha.x) {
        discard;
    }
    let rgb = mix(base.rgb, vec3<f32>(1.0, 0.0, 0.0), input.highlight);
    let opacity = select(1.0, base.a * input.alpha.y, input.alpha.y > 0.0);
    return vec4<f32>(rgb, opacity);
}

struct LineIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(input: LineIn) -> LineOut {
    var out: LineOut;
    out.clip = uniforms.view_proj * vec4<f32>(input.position, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_line(input: LineOut) -> @location(0) vec4<f32> {
    return input.color;
}
