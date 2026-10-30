//! An app's icon as Explorer draws it: the Shell's `IShellItemImageFactory`
//! on the `.exe`, read back as pixels and saved as a PNG (PwrSnap's
//! `--extract-app-icon` does the same). The Shell and GDI calls are COM and
//! Win32 FFI, so this module allows `unsafe` for them, each with a `SAFETY`
//! comment.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY,
};
use windows::core::PCWSTR;

use crate::icon_pixels::{png_from_rgba, rgba_from_premultiplied_bgra};

/// The largest side the Shell is asked for: Windows' icons stop at 256.
const MAX_SIDE: u32 = 256;

/// The file `app` names, when it is a path to one. Windows has no bundle
/// ids, and the app's scan already names each browser's `.exe`, so a name
/// alone finds nothing here.
pub fn installed_app(app: &str) -> Option<PathBuf> {
    let path = Path::new(app.trim());
    (path.is_absolute() && path.is_file()).then(|| path.to_path_buf())
}

/// The icon Explorer shows for `path`, as a PNG about `pixels` square (the
/// Shell may hand back a larger one; the badge scales it). `None` when the
/// Shell cannot draw it.
pub fn icon_png(path: &Path, pixels: u32) -> Option<Vec<u8>> {
    let side = pixels.clamp(16, MAX_SIDE) as i32;
    let _com = Com::init();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.ok()?;
    let flags = SIIGBF(SIIGBF_ICONONLY.0 | SIIGBF_BIGGERSIZEOK.0);
    // SAFETY: a plain COM call on an interface we hold; the bitmap it
    // returns is ours to delete, which `Bitmap` does.
    let bitmap = Bitmap(unsafe { factory.GetImage(SIZE { cx: side, cy: side }, flags) }.ok()?);
    let (width, height, bgra) = bitmap.pixels()?;
    png_from_rgba(&rgba_from_premultiplied_bgra(&bgra), width, height)
}

/// COM on this thread for as long as it lives. GPUI's background threads
/// may have it already, in either model; then nothing is undone.
struct Com(bool);

impl Com {
    fn init() -> Self {
        // SAFETY: no reserved pointer; balanced by `Drop` when it succeeds.
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        Self(result.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: pairs the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
    }
}

/// A bitmap the Shell handed over, deleted on drop.
struct Bitmap(HBITMAP);

impl Bitmap {
    /// Its width, height and 32-bit pixels, top-down rows of BGRA.
    fn pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let mut info = BITMAP::default();
        // SAFETY: `info` is a `BITMAP` and its size is passed with it.
        let read = unsafe {
            GetObjectW(
                HGDIOBJ(self.0.0),
                size_of::<BITMAP>() as i32,
                Some((&raw mut info).cast::<c_void>()),
            )
        };
        let (width, height) = (info.bmWidth, info.bmHeight.abs());
        if read == 0 || width <= 0 || height <= 0 || width > 1024 || height > 1024 {
            return None;
        }
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative: top-down rows, as a PNG wants them.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0_u8; width as usize * height as usize * 4];
        // SAFETY: the memory DC is deleted below; `bgra` holds exactly
        // `height` rows of `width` 32-bit pixels, the format `header` asks
        // for.
        let copied = unsafe {
            let dc = CreateCompatibleDC(None);
            let copied = GetDIBits(
                dc,
                self.0,
                0,
                height as u32,
                Some(bgra.as_mut_ptr().cast::<c_void>()),
                &raw mut header,
                DIB_RGB_COLORS,
            );
            let _ = DeleteDC(dc);
            copied
        };
        (copied == height).then_some((width as u32, height as u32, bgra))
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: the Shell's bitmap is ours and used nowhere else.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Width and height from a PNG's IHDR chunk.
    fn png_size(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        let word = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        (word(16), word(20))
    }

    fn explorer() -> PathBuf {
        let windir = std::env::var_os("WINDIR").expect("WINDIR");
        Path::new(&windir).join("explorer.exe")
    }

    #[test]
    fn an_exe_is_found_by_its_path_only() {
        let explorer = explorer();
        assert_eq!(
            installed_app(explorer.to_str().unwrap()).as_ref(),
            Some(&explorer)
        );
        assert_eq!(installed_app("Explorer"), None);
        assert_eq!(installed_app(r"C:\No Such Folder Example\app.exe"), None);
        assert_eq!(installed_app("  "), None);
    }

    #[test]
    fn explorer_has_an_icon_off_the_main_thread() {
        let explorer = explorer();
        let png = std::thread::spawn(move || icon_png(&explorer, 32))
            .join()
            .unwrap()
            .expect("explorer.exe's icon");
        let (width, height) = png_size(&png);
        assert!(width >= 32 && height >= 32, "{width}x{height}");
    }

    #[test]
    fn a_missing_file_has_no_icon() {
        assert_eq!(
            icon_png(Path::new(r"C:\No Such Folder Example\app.exe"), 32),
            None
        );
    }
}
