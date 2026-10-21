//! Checks made from outside the app, through the OS's own tools: which
//! top-level windows a process has, and a screenshot of one.
//!
//! - macOS: CoreGraphics' window list (`CGWindowListCopyWindowInfo`) from
//!   JavaScript for Automation, and `screencapture -l`. Neither needs the
//!   Accessibility permission; without Screen Recording the list omits
//!   titles and the capture shows the desktop.
//! - Windows: UI Automation from PowerShell, and a capture of the window's
//!   rectangle.
//! - Linux (X11, as under Xvfb): `xwininfo` and ImageMagick's `import`.

use std::{path::Path, process::Command};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

/// A top-level window as the OS reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct OsWindow {
    /// The OS's id: a CGWindowID, an HWND, or an X window id.
    pub id: u64,
    /// `None` where the OS withholds it (macOS without Screen Recording).
    pub title: Option<String>,
    pub width: f64,
    pub height: f64,
}

/// The visible top-level windows `pid` owns.
pub fn windows(pid: u32) -> Result<Vec<OsWindow>> {
    let json = if cfg!(target_os = "macos") {
        output(Command::new("osascript").args([
            "-l",
            "JavaScript",
            "-e",
            MACOS_WINDOWS,
            &pid.to_string(),
        ]))?
    } else if cfg!(windows) {
        output(Command::new("powershell").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &WINDOWS_WINDOWS.replace("{pid}", &pid.to_string()),
        ]))?
    } else {
        return linux_windows(pid);
    };
    parse(&json)
}

/// Saves a PNG of `pid`'s largest window to `path`.
pub fn screenshot(pid: u32, path: &Path) -> Result<()> {
    let window = windows(pid)?
        .into_iter()
        .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        .context("the process has no window to capture")?;
    let path_arg = path.display().to_string();
    if cfg!(target_os = "macos") {
        output(Command::new("screencapture").args([
            "-x",
            "-o",
            &format!("-l{}", window.id),
            &path_arg,
        ]))?;
    } else if cfg!(windows) {
        output(
            Command::new("powershell").args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &WINDOWS_CAPTURE
                    .replace("{hwnd}", &window.id.to_string())
                    .replace("{path}", &path_arg.replace('\'', "''")),
            ]),
        )?;
    } else {
        // The root's framebuffer, cropped to the window: under Xvfb a
        // window drawn through Vulkan captures black by its own id.
        let info = output(Command::new("xwininfo").args(["-id", &format!("0x{:x}", window.id)]))?;
        let field = |name: &str| {
            info.lines()
                .find_map(|l| l.trim().strip_prefix(name))
                .and_then(|v| v.trim().parse::<i64>().ok())
                .with_context(|| format!("xwininfo gave no {name}"))
        };
        let crop = format!(
            "{}x{}+{}+{}",
            field("Width:")?,
            field("Height:")?,
            field("Absolute upper-left X:")?,
            field("Absolute upper-left Y:")?
        );
        output(Command::new("import").args(["-window", "root", "-crop", &crop, &path_arg]))?;
    }
    Ok(())
}

fn output(command: &mut Command) -> Result<String> {
    let out = command
        .output()
        .with_context(|| format!("run {command:?}"))?;
    if !out.status.success() {
        bail!(
            "{command:?} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn parse(json: &str) -> Result<Vec<OsWindow>> {
    if json.is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(json).with_context(|| format!("bad JSON {json:?}"))?;
    // PowerShell writes a lone object, not an array of one.
    let list = match value {
        Value::Array(list) => list,
        other => vec![other],
    };
    Ok(list
        .iter()
        .filter_map(|w| {
            Some(OsWindow {
                id: w["id"].as_u64()?,
                title: w["title"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned),
                width: w["width"].as_f64()?,
                height: w["height"].as_f64()?,
            })
        })
        .collect())
}

/// Layer 0 is where an app's document windows live; menus and the status
/// item are on other layers.
const MACOS_WINDOWS: &str = r#"
function run(argv) {
  ObjC.import("CoreGraphics");
  const pid = Number(argv[0]);
  const list = ObjC.castRefToObject(
    $.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0));
  const out = [];
  for (let i = 0; i < list.count; i++) {
    const w = ObjC.deepUnwrap(list.objectAtIndex(i));
    if (w.kCGWindowOwnerPID !== pid || w.kCGWindowLayer !== 0) continue;
    out.push({ id: w.kCGWindowNumber, title: w.kCGWindowName || null,
               width: w.kCGWindowBounds.Width, height: w.kCGWindowBounds.Height });
  }
  return JSON.stringify(out);
}
"#;

const WINDOWS_WINDOWS: &str = r#"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$ae = [System.Windows.Automation.AutomationElement]
$cond = New-Object System.Windows.Automation.PropertyCondition($ae::ProcessIdProperty, {pid})
$found = $ae::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
$out = @(foreach ($w in $found) {
  $r = $w.Current.BoundingRectangle
  [pscustomobject]@{ id = [uint64]$w.Current.NativeWindowHandle; title = $w.Current.Name; width = $r.Width; height = $r.Height }
})
ConvertTo-Json -Compress -InputObject $out
"#;

const WINDOWS_CAPTURE: &str = r#"
Add-Type -AssemblyName UIAutomationClient, System.Drawing
$w = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]{hwnd})
$r = $w.Current.BoundingRectangle
$bmp = New-Object System.Drawing.Bitmap ([int]$r.Width), ([int]$r.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen([int]$r.X, [int]$r.Y, 0, 0, $bmp.Size)
$bmp.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png)
"#;

/// `xwininfo -root -children` lists the root's children; a window manager
/// would reparent them, so `-tree` is used, filtered to named windows with
/// the process id in `_NET_WM_PID`.
fn linux_windows(pid: u32) -> Result<Vec<OsWindow>> {
    let tree = output(Command::new("xwininfo").args(["-root", "-tree"]))?;
    let mut found = Vec::new();
    for line in tree.lines() {
        // `     0x1a00003 "PwrRoute": ("pwrroute" "pwrroute")  1240x820+0+0  +0+0`
        let line = line.trim_start();
        let Some((id, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some(id) = id
            .strip_prefix("0x")
            .and_then(|h| u64::from_str_radix(h, 16).ok())
        else {
            continue;
        };
        let Some(title) = rest
            .strip_prefix('"')
            .and_then(|r| r.split_once('"'))
            .map(|(t, _)| t.to_owned())
        else {
            continue;
        };
        let Some((width, height)) = rest.split_whitespace().find_map(geometry_size) else {
            continue;
        };
        let owner =
            output(Command::new("xprop").args(["-id", &format!("0x{id:x}"), "_NET_WM_PID"]))
                .unwrap_or_default();
        if owner.rsplit(' ').next() == Some(&pid.to_string()) {
            found.push(OsWindow {
                id,
                title: Some(title),
                width,
                height,
            });
        }
    }
    Ok(found)
}

/// The size from an X geometry, `1240x820+340+130` or `1240x820-5+0`.
fn geometry_size(token: &str) -> Option<(f64, f64)> {
    let size = token.split(['+', '-']).next()?;
    let (width, height) = size.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_lists_parse_from_every_os() {
        let mac = r#"[{"id":7,"title":"PwrRoute","width":1240,"height":820},{"id":8,"title":null,"width":1,"height":1}]"#;
        let windows = parse(mac).unwrap();
        assert_eq!(windows[0].title.as_deref(), Some("PwrRoute"));
        assert_eq!(windows[1].title, None);
        // PowerShell's ConvertTo-Json writes a single window as an object.
        let win = r#"{"id":132456,"title":"PwrRoute","width":1240.0,"height":820.0}"#;
        assert_eq!(parse(win).unwrap()[0].id, 132_456);
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn x_geometries_give_their_size_whatever_the_offset() {
        assert_eq!(geometry_size("1240x820+340+130"), Some((1240.0, 820.0)));
        assert_eq!(geometry_size("1240x820-5+0"), Some((1240.0, 820.0)));
        assert_eq!(geometry_size("(\"pwrroute\""), None);
    }
}
