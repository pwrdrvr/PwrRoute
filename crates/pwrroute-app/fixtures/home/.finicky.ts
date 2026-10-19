// The Finicky config of design option 1b: contrived, for the app's tests and
// screenshots. `fixtures::config_in_charge` draws these same handlers.
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
