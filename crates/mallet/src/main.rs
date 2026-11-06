//! Mallet: a Hammer-style level editor for Source engine VMF maps.
//!
//! Pass a `.vmf` path as the first argument to open it on start, or drop one onto the window.
//! AI agents can drive the editor through its MCP server; see `mcp`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod config;
mod document;
mod editor;
mod input;
mod keymap;
mod mcp;
mod netgraph;
mod render3d;
mod scene;
mod tools;
mod view2d;
mod view3d;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mallet")
            .with_inner_size([1600.0, 1000.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Mallet",
        options,
        Box::new(move |cc| Ok(Box::new(app::MalletApp::new(cc, path)))),
    )
}
