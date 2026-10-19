// The Finicky config of design option 1b: contrived, for the app's tests and
// screenshots. The Rules screen draws what the engine reads in it.
import type { FinickyConfig } from "/Applications/Finicky.app/Contents/Resources/finicky.d.ts";

export default {
  defaultBrowser: "Google Chrome",
  handlers: [
    {
      match: (url, { opener }) => url.host.includes("workplace"),
      browser: "Firefox",
    },
    {
      match: (url, { opener }) => opener?.name.includes("Slack") || false,
      browser: "Firefox",
    },
    {
      match: "bsky.app/*",
      browser: "Firefox",
    },
    {
      match: ["google.com/*", "*.google.com*"],
      browser: "Google Chrome",
    },
  ],
} satisfies FinickyConfig;
