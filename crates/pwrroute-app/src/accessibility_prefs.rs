//! Live OS preferences. Palette values belong to theme; this module only
//! follows the system and tells GPUI and the palette seam when it changes.

use gpui_kit::{App, Global};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessibilityPrefs {
    pub reduce_motion: bool,
    pub increase_contrast: bool,
}
impl Global for AccessibilityPrefs {}

pub fn current(cx: &App) -> AccessibilityPrefs {
    AccessibilityPrefs {
        reduce_motion: cx.reduce_motion(),
        increase_contrast: crate::theme::increase_contrast(),
    }
}

fn refresh(contrast: Option<bool>, cx: &mut App) {
    let old = cx
        .try_global::<AccessibilityPrefs>()
        .copied()
        .unwrap_or_default();
    gpui_kit::base::apply_system_reduce_motion(cx);
    if let Some(contrast) = contrast {
        crate::theme::set_increase_contrast(contrast);
    }
    let new = current(cx);
    cx.set_global(new);
    if old != new {
        cx.refresh_windows();
    }
}

pub fn install(cx: &mut App) {
    // Tests and the fixture renderer control motion themselves and must never
    // be woken by a real OS response or an endless timer.
    if cx
        .background_executor()
        .scheduler_executor()
        .scheduler()
        .as_test()
        .is_some()
    {
        return;
    }
    refresh(pwrroute_os::accessibility::increase_contrast(), cx);
    // Resident apps must notice changes with no window open. These native
    // reads are cheap; polling also catches settings changed during sleep.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            cx.update(|cx| refresh(pwrroute_os::accessibility::increase_contrast(), cx));
        }
    })
    .detach();
    #[cfg(target_os = "linux")]
    follow_portal(cx);
}

#[cfg(target_os = "linux")]
fn follow_portal(cx: &mut App) {
    use ashpd::desktop::settings::{Contrast, Settings};
    use futures::StreamExt as _;
    cx.spawn(async move |cx| {
        let settings = Settings::new().await.ok()?;
        // Subscribe before the first read so a change cannot be missed.
        let mut changes = settings.receive_contrast_changed().await.ok()?;
        if let Ok(value) = settings.contrast().await {
            cx.update(|cx| refresh(Some(matches!(value, Contrast::High)), cx));
        }
        while let Some(value) = changes.next().await {
            cx.update(|cx| refresh(Some(matches!(value, Contrast::High)), cx));
        }
        Some(())
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui_kit::test]
    fn accessibility_refresh_preserves_application_motion_override(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            refresh(None, cx);
            assert!(current(cx).reduce_motion);
            assert!(cx.global::<AccessibilityPrefs>().reduce_motion);
        });
    }
}
