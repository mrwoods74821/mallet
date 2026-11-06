//! Mallet: a Hammer-style level editor for Source engine VMF maps.
//!
//! Pass a `.vmf` path as the first argument to open it on start, or drop one onto the window.
//! AI agents can drive the editor through its MCP server; see `mcp`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branding;
mod canvas;
mod compile;
mod config;
mod document;
mod editor;
mod faces;
mod input;
mod keymap;
mod mcp;
mod netgraph;
mod properties;
mod render3d;
mod scene;
mod tools;
mod view2d;
mod view3d;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let present_mode = match std::env::var("MALLET_PRESENT").as_deref() {
        Ok("fifo") => eframe::wgpu::PresentMode::Fifo,
        Ok("mailbox") => eframe::wgpu::PresentMode::Mailbox,
        Ok("immediate") => eframe::wgpu::PresentMode::Immediate,
        Ok("novsync") => eframe::wgpu::PresentMode::AutoNoVsync,
        _ => eframe::wgpu::PresentMode::AutoVsync,
    };
    let latency = std::env::var("MALLET_FRAME_LATENCY")
        .ok()
        .and_then(|value| value.parse().ok())
        .or(Some(1));
    // eframe keeps the window's size and place and every panel's position and size in
    // %APPDATA%\<app id>\data. MALLET_PROFILE picks a separate store, so test copies do
    // not move your windows around.
    let app_id = config::app_folder();
    let options = eframe::NativeOptions {
        persist_window: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("Mallet")
            .with_app_id(app_id.clone())
            .with_icon(std::sync::Arc::new(branding::window_icon()))
            .with_inner_size([1600.0, 1000.0])
            .with_drag_and_drop(true),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode,
            desired_maximum_frame_latency: latency,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        &app_id,
        options,
        Box::new(move |cc| Ok(Box::new(app::MalletApp::new(cc, path)))),
    )
}
