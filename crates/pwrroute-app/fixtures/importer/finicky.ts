// The Finicky config of the importer's design (Finicky Importer 2a):
// contrived, for the app's tests and screenshots. One rule of each verdict:
// wildcards and a string rewrite import, functions and a regex stay JS, and
// a helper-function matcher earns a suggestion.
const hostnameIs = (url: URL, host: string): boolean => url.hostname === host;

export default {
  defaultBrowser: "Google Chrome",
  rewrite: [
    {
      match: "x.com/*",
      url: "https://xcancel.com/",
    },
    {
      match: "*",
      url: (url: URL) => {
        url.searchParams.delete("utm_source");
        return url;
      },
    },
  ],
  handlers: [
    {
      match: (url: URL) => url.host.includes("workplace"),
      browser: "Firefox",
    },
    {
      match: (url: URL) => hostnameIs(url, "meet.google.com"),
      browser: { name: "Firefox", profile: "default-release" },
    },
    {
      match: "bsky.app/*",
      browser: "Firefox",
    },
    {
      match: /^https:\/\/news\.ycombinator\.com/,
      browser: "Firefox",
    },
    {
      match: ["google.com/*", "*.google.com*"],
      browser: "Google Chrome",
    },
    {
      match: "*.atlassian.net/*",
      browser: "Google Chrome:Work",
    },
  ],
};
