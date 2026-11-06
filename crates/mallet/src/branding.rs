//! Mallet's icon, a white mallet on a black circle, built into the program from the files
//! `branding/make_icons.py` draws. The Windows executable gets the `.ico` through
//! `build.rs`; this module gives the window, the menu bar, the About window and the MCP
//! server the same picture.

use base64::Engine;
use eframe::egui;

/// The icon at 256 pixels, for the window and taskbar.
const ICON_256: &[u8] = include_bytes!("../../../branding/mallet-256.png");
/// The icon at 64 pixels, for the menu bar and MCP clients.
const ICON_64: &[u8] = include_bytes!("../../../branding/mallet-64.png");

/// The icon the window and taskbar show.
pub fn window_icon() -> egui::IconData {
    let (rgba, [width, height]) = decode(ICON_256);
    egui::IconData { rgba, width, height }
}

/// The icon as an egui texture, for drawing it in the interface.
pub fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let (rgba, [width, height]) = decode(ICON_256);
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
    // Mipmaps keep the icon crisp when drawn small, as in the menu bar.
    let options = egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
    ctx.load_texture("mallet_logo", image, options)
}

/// The small icon as a `data:` URL, as MCP clients take server icons.
pub fn icon_data_url() -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(ICON_64)
    )
}

/// Decodes a built-in PNG into straight (not premultiplied) RGBA.
fn decode(png: &[u8]) -> (Vec<u8>, [u32; 2]) {
    let pixmap = tiny_skia::Pixmap::decode_png(png).expect("the built-in icon is a valid PNG");
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    (rgba, [pixmap.width(), pixmap.height()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_a_black_circle_with_a_white_mallet() {
        let (rgba, [width, height]) = decode(ICON_256);
        assert_eq!([width, height], [256, 256]);
        let pixel = |x: usize, y: usize| &rgba[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4];
        assert_eq!(pixel(0, 0)[3], 0, "the corners are transparent");
        assert_eq!(pixel(20, 128), [0, 0, 0, 255], "the circle is black");
        assert_eq!(
            pixel(128, 128),
            [255, 255, 255, 255],
            "the mallet crosses the centre in white"
        );
        assert!(icon_data_url().starts_with("data:image/png;base64,iVBOR"));
    }
}
