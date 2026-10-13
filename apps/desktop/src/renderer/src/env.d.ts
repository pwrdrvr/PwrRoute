/// <reference types="vite/client" />
import type { PwrRouteApi } from "../../preload/index";

declare global {
  interface Window {
    readonly pwrroute: PwrRouteApi;
  }
}
