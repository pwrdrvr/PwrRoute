import { describe, expect, it } from "vitest";
import { MACOS_TRAFFIC_LIGHT_POSITION, mainWindowChromeOptions } from "./window-chrome";

describe("mainWindowChromeOptions", () => {
  it("insets the macOS traffic lights at the family position", () => {
    expect(mainWindowChromeOptions("darwin")).toEqual({
      titleBarStyle: "hiddenInset",
      trafficLightPosition: MACOS_TRAFFIC_LIGHT_POSITION
    });
  });

  it("gives Windows a caption-button overlay as tall as the strip", () => {
    const options = mainWindowChromeOptions("win32");
    expect(options.titleBarStyle).toBe("hidden");
    expect(options.titleBarOverlay).toMatchObject({ height: 40 });
  });

  it("keeps the native frame on Linux", () => {
    expect(mainWindowChromeOptions("linux")).toEqual({});
  });
});
