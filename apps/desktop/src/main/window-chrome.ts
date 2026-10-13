import type { BrowserWindowConstructorOptions } from "electron";

type ChromeOptions = Pick<
  BrowserWindowConstructorOptions,
  "titleBarOverlay" | "titleBarStyle" | "trafficLightPosition"
>;

/** Family chrome: stoplights at (16, 13) inside the 40px strip. */
export const MACOS_TRAFFIC_LIGHT_POSITION = { x: 16, y: 13 } as const;

/** The strip's height, which the Windows caption-button overlay matches. */
export const TITLE_STRIP_HEIGHT = 40;

/**
 * Platform chrome for the main window. macOS insets its traffic lights into
 * the painted strip, and Windows lets the OS paint caption buttons into an
 * overlay on the right. Linux keeps its native frame for now: a frameless
 * Linux window has no caption buttons until the renderer paints its own, as
 * PwrGit's WindowControls does.
 */
export function mainWindowChromeOptions(
  platform: NodeJS.Platform = process.platform
): ChromeOptions {
  if (platform === "darwin") {
    return {
      titleBarStyle: "hiddenInset",
      trafficLightPosition: { ...MACOS_TRAFFIC_LIGHT_POSITION }
    };
  }
  if (platform === "win32") {
    return {
      titleBarStyle: "hidden",
      titleBarOverlay: { color: "#050505", symbolColor: "#f7f3eb", height: TITLE_STRIP_HEIGHT }
    };
  }
  return {};
}
