//! Turning the pixels an OS hands back for an app's icon into a PNG. Plain
//! code, so it is tested on every platform; Windows' `app_icon` feeds it the
//! Shell's bitmap.

/// The Shell's 32-bit pixels (top-down rows, BGRA, alpha premultiplied) as
/// straight RGBA. An icon with no alpha at all (every alpha 0, an old icon
/// without a mask channel) is opaque.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn rgba_from_premultiplied_bgra(bgra: &[u8]) -> Vec<u8> {
    let has_alpha = bgra.chunks_exact(4).any(|px| px[3] != 0);
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
        if !has_alpha {
            rgba.extend_from_slice(&[r, g, b, 255]);
            continue;
        }
        let straight = |c: u8| match a {
            0 => 0,
            255 => c,
            _ => ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8,
        };
        rgba.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
    }
    rgba
}

/// `rgba` (`width` × `height`, straight alpha) as a PNG.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn png_from_rgba(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(png)
}

/// Width and height from a PNG's IHDR chunk, for the tests here and in
/// each platform's `app_icon`.
#[cfg(test)]
pub(crate) fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    let word = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (word(16), word(20))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplied_pixels_come_back_straight() {
        // Opaque red, half-transparent white, fully transparent.
        let bgra = [0, 0, 255, 255, 128, 128, 128, 128, 0, 0, 0, 0];
        assert_eq!(
            rgba_from_premultiplied_bgra(&bgra),
            [255, 0, 0, 255, 255, 255, 255, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn an_icon_without_alpha_is_opaque() {
        let bgra = [10, 20, 30, 0, 40, 50, 60, 0];
        assert_eq!(
            rgba_from_premultiplied_bgra(&bgra),
            [30, 20, 10, 255, 60, 50, 40, 255]
        );
    }

    #[test]
    fn the_png_has_the_size_given() {
        let png = png_from_rgba(&[0; 2 * 3 * 4], 2, 3).expect("a PNG");
        assert_eq!(png_size(&png), (2, 3));
        assert_eq!(png_from_rgba(&[0; 4], 2, 2), None, "too few pixels");
        assert_eq!(png_from_rgba(&[], 0, 0), None);
    }
}
