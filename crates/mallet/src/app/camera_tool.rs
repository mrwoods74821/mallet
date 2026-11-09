//! The Camera tool. In a 3D view a left drag turns the camera and a right drag slides it.
//! In a 2D view a left click zooms in and a right click zooms out, around the cursor.

use eframe::egui::{PointerButton, Response};

use super::MalletApp;
use crate::editor::{Editor, Tool};

impl MalletApp {
    /// Runs the Camera tool in a 3D view: a left drag turns the camera, and a right drag
    /// slides it. Only the view being dragged gets the drag, so the shared camera moves once.
    pub(super) fn camera_tool_3d(&mut self, response: &Response, editor: &mut Editor) {
        if editor.tool != Tool::Camera {
            return;
        }
        if response.dragged_by(PointerButton::Primary) {
            editor.camera.look_by(response.drag_delta());
        } else if response.dragged_by(PointerButton::Secondary) {
            editor.camera.pan_by(response.drag_delta());
        }
    }
}
