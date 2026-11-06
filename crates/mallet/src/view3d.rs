//! The 3D camera viewport. Draws wireframes for now; textured rendering comes with wgpu.

use eframe::egui::{self, Color32, Pos2, Rect, ViewportCommand};
use glam::DVec3;
use mallet_vmf::geometry::Bounds;

use crate::canvas::Canvas;
use crate::keymap::{Action, KeyMap};
use crate::scene::{box_edges, Overlay, Scene, TOOL_COLOR};

/// World units per second, matching Hammer's default forward speed.
const MOVE_SPEED: f64 = 1000.0;
/// Degrees per second for arrow-key turning.
const TURN_SPEED: f64 = 90.0;
/// Degrees per pixel of mouse movement in mouselook.
const LOOK_SENSITIVITY: f64 = 0.2;
/// World units the camera moves per mouse wheel notch.
const WHEEL_STEP: f64 = 64.0;
const POINTS_PER_NOTCH: f64 = 50.0;
/// Distance of the near clipping plane.
const NEAR: f64 = 1.0;
const MAX_PITCH: f64 = 89.0;

/// A free-flying first-person camera in Source's Z-up coordinates.
pub struct Camera3D {
    pub position: DVec3,
    /// Degrees around Z; 0 looks along +X.
    pub yaw: f64,
    /// Degrees above the horizon.
    pub pitch: f64,
    /// Horizontal field of view in degrees.
    pub fov: f64,
    mouse_look: bool,
}

impl Camera3D {
    pub fn new() -> Self {
        Self {
            position: DVec3::new(-512.0, -512.0, 512.0),
            yaw: 45.0,
            pitch: -30.0,
            fov: 90.0,
            mouse_look: false,
        }
    }

    pub fn forward(&self) -> DVec3 {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        DVec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin())
    }

    pub fn right(&self) -> DVec3 {
        let yaw = self.yaw.to_radians();
        DVec3::new(yaw.sin(), -yaw.cos(), 0.0)
    }

    /// Moves the camera back from the bounds' centre until all of it is in view.
    pub fn look_at(&mut self, bounds: &Bounds) {
        if bounds.is_empty() {
            return;
        }
        let radius = (bounds.size().length() * 0.5).max(128.0);
        self.position = bounds.center() - self.forward() * radius * 1.5;
    }

    fn set_mouse_look(&mut self, ctx: &egui::Context, enabled: bool) {
        self.mouse_look = enabled;
        let grab = if enabled {
            egui::CursorGrab::Confined
        } else {
            egui::CursorGrab::None
        };
        ctx.send_viewport_cmd(ViewportCommand::CursorGrab(grab));
        ctx.send_viewport_cmd(ViewportCommand::CursorVisible(!enabled));
    }

    /// Applies keyboard, wheel and mouselook input. Returns whether the camera needs
    /// another frame because it is moving or in mouselook.
    ///
    /// Input is only read while the viewport is hovered or mouselook is on, like Hammer,
    /// where Z toggles mouselook and Escape leaves it.
    pub fn update(&mut self, ctx: &egui::Context, hovered: bool, keymap: &KeyMap) -> bool {
        if !hovered && !self.mouse_look {
            return false;
        }
        let (toggle, escape) = ctx.input(|input| {
            (
                keymap.pressed(input, Action::ToggleMouseLook),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if toggle || (escape && self.mouse_look) {
            self.set_mouse_look(ctx, toggle && !self.mouse_look);
        }

        let mut active = self.mouse_look;
        ctx.input(|input| {
            let dt = f64::from(input.stable_dt).min(0.1);
            if self.mouse_look {
                for event in &input.events {
                    if let egui::Event::MouseMoved(delta) = event {
                        self.yaw -= f64::from(delta.x) * LOOK_SENSITIVITY;
                        self.pitch -= f64::from(delta.y) * LOOK_SENSITIVITY;
                    }
                }
            }

            let turn = |positive: Action, negative: Action| {
                f64::from(u8::from(keymap.held(input, positive))) - f64::from(u8::from(keymap.held(input, negative)))
            };
            let yaw_input = turn(Action::TurnLeft, Action::TurnRight);
            let pitch_input = turn(Action::LookUp, Action::LookDown);
            self.yaw += yaw_input * TURN_SPEED * dt;
            self.pitch = (self.pitch + pitch_input * TURN_SPEED * dt).clamp(-MAX_PITCH, MAX_PITCH);
            self.yaw = self.yaw.rem_euclid(360.0);

            let forward = self.forward();
            let right = self.right();
            let mut wish = DVec3::ZERO;
            wish += forward * turn(Action::MoveForward, Action::MoveBack);
            wish += right * turn(Action::StrafeRight, Action::StrafeLeft);
            if wish != DVec3::ZERO {
                self.position += wish.normalize() * MOVE_SPEED * dt;
            }

            let scroll = f64::from(input.raw_scroll_delta.y);
            if hovered && scroll != 0.0 {
                self.position += forward * WHEEL_STEP * scroll / POINTS_PER_NOTCH;
            }
            active |= wish != DVec3::ZERO || yaw_input != 0.0 || pitch_input != 0.0;
        });
        active
    }

    /// Whether mouselook is on.
    pub fn mouse_look(&self) -> bool {
        self.mouse_look
    }

    /// Focal length in pixels for a viewport of this size.
    fn focal(&self, rect: Rect) -> f64 {
        f64::from(rect.width()) * 0.5 / (self.fov.to_radians() * 0.5).tan()
    }

    /// The world-space ray from the camera through a point of the viewport.
    pub fn ray(&self, rect: Rect, pos: Pos2) -> (DVec3, DVec3) {
        let forward = self.forward();
        let right = self.right();
        let up = right.cross(forward);
        let focal = self.focal(rect);
        let x = f64::from(pos.x - rect.center().x) / focal;
        let y = -f64::from(pos.y - rect.center().y) / focal;
        (self.position, (forward + right * x + up * y).normalize())
    }

    /// Draws every wireframe in perspective, clipping edges at the near plane. Selected
    /// meshes are drawn last in red at the overlay's move offset, then the block tool's box.
    pub fn draw(&self, canvas: &mut dyn Canvas, rect: Rect, scene: &Scene, active: bool, overlay: &Overlay) {
        canvas.fill(rect, Color32::BLACK);
        let forward = self.forward();
        let right = self.right();
        let up = right.cross(forward);
        let focal = self.focal(rect);
        let center = rect.center();
        let to_camera = |point: DVec3| {
            let offset = point - self.position;
            DVec3::new(offset.dot(right), offset.dot(up), offset.dot(forward))
        };
        let to_screen = |camera: DVec3| {
            Pos2::new(
                center.x + (camera.x / camera.z * focal) as f32,
                center.y - (camera.y / camera.z * focal) as f32,
            )
        };
        let draw_edges = |canvas: &mut dyn Canvas, edges: &[[DVec3; 2]], offset: DVec3, color: Color32| {
            let mut segments = Vec::with_capacity(edges.len());
            for [a, b] in edges {
                let (mut a, mut b) = (to_camera(*a + offset), to_camera(*b + offset));
                if a.z < NEAR && b.z < NEAR {
                    continue;
                }
                if a.z < NEAR {
                    a = a + (b - a) * ((NEAR - a.z) / (b.z - a.z));
                } else if b.z < NEAR {
                    b = b + (a - b) * ((NEAR - b.z) / (a.z - b.z));
                }
                segments.push([to_screen(a), to_screen(b)]);
            }
            canvas.lines(&segments, color);
        };

        let unselected = scene.meshes.iter().filter(|mesh| !mesh.selected);
        let selected = scene.meshes.iter().filter(|mesh| mesh.selected);
        for mesh in unselected.chain(selected) {
            let (color, offset) = mesh.appearance(overlay);
            let radius = mesh.bounds.size().length() * 0.5;
            if to_camera(mesh.bounds.center() + offset).z + radius < NEAR {
                continue;
            }
            draw_edges(canvas, &mesh.edges, offset, color);
        }
        if let Some(block) = &overlay.block {
            draw_edges(canvas, &box_edges(block), DVec3::ZERO, TOOL_COLOR);
        }

        let label_color = if active { Color32::WHITE } else { Color32::GRAY };
        canvas.label(rect.left_top() + egui::vec2(6.0, 4.0), "camera (wireframe)", label_color);
    }
}
