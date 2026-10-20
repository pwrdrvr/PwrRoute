//! The tray icon, rasterized at startup from the PwrRoute mark
//! (`assets/icons/mark.svg`), so it can never drift from the mark.
//!
//! macOS gets a template image: black strokes on transparent, which AppKit
//! tints for the menu bar's appearance. The mark's back route keeps its 0.55
//! opacity, so the template has the design's two tones. Windows and Linux
//! have no template images, so they get the mark in the accent color.

use resvg::{tiny_skia, usvg};

const MARK: &str = include_str!("../../assets/icons/mark.svg");

/// The accent, `--accent` in `design/pwrroute.css`.
const ACCENT: &str = "#ff8a1f";

/// An RGBA bitmap, row-major, straight (not premultiplied) alpha.
#[derive(Debug)]
pub struct Bitmap {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The macOS menu bar icon: a 22pt square at 2x with the mark drawn 16pt
/// tall in its middle, as in the design's 1a. tray-icon caps the image at
/// 22pt high, so 44px is exactly @2x.
pub fn template() -> Bitmap {
    render("#000", 44, 32)
}

/// The Windows notification area and Linux StatusNotifierItem icon.
pub fn colored() -> Bitmap {
    render(ACCENT, 32, 30)
}

/// The mark in `stroke`, `mark` px square, centered on a `canvas` px square.
fn render(stroke: &str, canvas: u32, mark: u32) -> Bitmap {
    let svg = MARK.replace("stroke=\"#000\"", &format!("stroke=\"{stroke}\""));
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())
        .expect("the embedded mark is valid SVG");
    let mut pixmap = tiny_skia::Pixmap::new(canvas, canvas).expect("a non-empty canvas");
    let size = tree.size();
    let scale = mark as f32 / size.width().max(size.height());
    let inset = (canvas - mark) as f32 / 2.0;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(inset, inset);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia stores premultiplied alpha; tray icons take straight alpha.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Bitmap {
        rgba,
        width: canvas,
        height: canvas,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(bitmap: &Bitmap) -> impl Iterator<Item = u8> + '_ {
        bitmap.rgba.chunks_exact(4).map(|px| px[3])
    }

    #[test]
    fn the_template_is_black_on_transparent() {
        let icon = template();
        assert_eq!((icon.width, icon.height), (44, 44));
        assert_eq!(icon.rgba.len(), 44 * 44 * 4);
        for px in icon.rgba.chunks_exact(4).filter(|px| px[3] > 0) {
            assert_eq!(&px[..3], &[0, 0, 0], "template pixels are black");
        }
        // Both tones are there: the solid route and the 0.55 one.
        let opaque = alpha(&icon).filter(|a| *a == 255).count();
        let back = alpha(&icon).filter(|a| (130..150).contains(a)).count();
        assert!(opaque > 50, "solid strokes: {opaque}");
        assert!(back > 20, "the faded route: {back}");
    }

    #[test]
    fn the_template_leaves_the_design_padding() {
        let icon = template();
        let inked = |x: u32, y: u32| icon.rgba[((y * icon.width + x) * 4 + 3) as usize] > 0;
        // The mark is 32px in a 44px canvas: 6px clear on every side.
        for i in 0..44 {
            for edge in 0..5 {
                assert!(!inked(i, edge) && !inked(i, 43 - edge));
                assert!(!inked(edge, i) && !inked(43 - edge, i));
            }
        }
    }

    #[test]
    fn the_colored_icon_uses_the_accent() {
        let icon = colored();
        assert_eq!((icon.width, icon.height), (32, 32));
        let solid = icon
            .rgba
            .chunks_exact(4)
            .find(|px| px[3] == 255)
            .expect("an opaque pixel");
        assert_eq!(&solid[..3], &[0xff, 0x8a, 0x1f]);
    }
}
