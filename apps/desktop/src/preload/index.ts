import { contextBridge } from "electron";

// The renderer's only window onto the host. Sandboxed preloads still see
// `process.platform`, which the strip needs to place its stoplight gutter.
const api = {
  platform: process.platform
} as const;

export type PwrRouteApi = typeof api;

contextBridge.exposeInMainWorld("pwrroute", api);
