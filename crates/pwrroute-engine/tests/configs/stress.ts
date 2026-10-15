// A config that touches every feature the Go and Rust hosts must agree on.
import type { FinickyConfig } from "/Applications/Finicky.app/Contents/Resources/finicky.d.ts";
import { WORK_DOMAINS, stripTracking } from "./helpers";

enum Profile {
  Work = "Profile 1",
  Personal = "Default",
}

const zoomJoin = /^https:\/\/(?<org>[\w-]+)\.zoom\.us\/j\/(?<id>\d+)/;

export default {
  defaultBrowser: "Safari",
  options: { urlShorteners: [] },
  rewrite: [
    { match: (url: URL) => url.searchParams.has("utm_source") || url.searchParams.has("fbclid"), url: (url: URL) => stripTracking(url) },
    { match: "http://*", url: (url: URL) => { url.protocol = "https:"; return url; } },
    {
      match: zoomJoin,
      url: (url: URL) => {
        const m = url.href.match(zoomJoin)!;
        return `zoommtg://zoom.us/join?confno=${m.groups!.id}&org=${m.groups!.org}`;
      },
    },
    { match: "*.medium.com/*", url: (url: URL) => "https://scribe.rip" + url.pathname },
    { match: "old.example.net/*", url: "https://new.example.net/" },
  ],
  handlers: [
    { match: (url: URL) => url.protocol === "zoommtg:", browser: "us.zoom.xos" },
    { match: finicky.matchHostnames(WORK_DOMAINS), browser: { name: "Google Chrome", profile: Profile.Work } },
    { match: finicky.matchDomains(["github.com"]), browser: { name: "Firefox", args: ["--new-tab"] } },
    { match: [/linear\.app/, "notion.so/*"], browser: { name: "Arc", openInBackground: true } },
    { match: /^https:\/\/(?<sub>\w+)\.youtube\.com/, browser: (url: URL) => (url.pathname.startsWith("/shorts") ? "Brave Browser" : { name: "Google Chrome", profile: Profile.Personal }) },
    { match: "*.figma.com/*", browser: { name: "/Applications/Figma.app", appType: "path" } },
    { match: "scribe.rip/*", browser: "Firefox" },
    { match: (url: URL) => url.hostname.endsWith(".local"), browser: null },
  ],
} satisfies FinickyConfig;
