// The config every E2E launch loads, so a run never reads the developer's
// own ~/.finicky.js. Contrived hosts only (example.com, .org, .net): URLs are
// private data, and test output must never carry a real one.
//
// The routing scenarios in tests/routing.rs assert these decisions; change
// both together.
export default {
  defaultBrowser: "Firefox",
  options: { urlShorteners: [] },
  rewrite: [
    {
      // rewrite[0]: drop a tracking parameter.
      match: (url) => url.searchParams.has("utm_source"),
      url: (url) => {
        url.searchParams.delete("utm_source");
        return url;
      },
    },
  ],
  handlers: [
    // handlers[0]
    { match: "https://work.example.com/*", browser: { name: "Google Chrome", profile: "Work" } },
    // handlers[1]
    { match: "*.example.org/*", browser: "Safari" },
    // handlers[2]: deliberately not opened.
    { match: "https://blocked.example.net/*", browser: null },
  ],
};
