import { join } from "node:path";
import { app, BrowserWindow, shell } from "electron";
import { mainWindowChromeOptions } from "./window-chrome";

// Scaffold only. The router, rules, Finicky config loading and the MCP server
// arrive with the app port; this window shows the family title strip.

function createMainWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 960,
    height: 640,
    minWidth: 640,
    minHeight: 400,
    show: false,
    backgroundColor: "#000000",
    title: "PwrRoute",
    ...mainWindowChromeOptions(),
    webPreferences: {
      preload: join(__dirname, "../preload/index.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true
    }
  });

  window.once("ready-to-show", () => window.show());

  // Links in the renderer never open in-app; hand them to the OS.
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("https://")) void shell.openExternal(url);
    return { action: "deny" };
  });

  const devUrl = process.env["ELECTRON_RENDERER_URL"];
  if (!app.isPackaged && devUrl !== undefined) {
    void window.loadURL(devUrl);
  } else {
    void window.loadFile(join(__dirname, "../renderer/index.html"));
  }
  return window;
}

void app.whenReady().then(() => {
  createMainWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createMainWindow();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});
