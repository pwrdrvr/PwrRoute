// handlerConfig from Finicky's packages/config-api/src/config.test.ts.
export default {
  defaultBrowser: "Firefox",
  handlers: [
    { match: "browser.spotify.com*", browser: "Spotify" },
    { match: ["github.com*", /amazon/], browser: "Google Chrome" },
    {
      match: ["figma.com*", "sketch.com*"],
      browser: { name: "Google Chrome", profile: "Design" },
    },
    {
      match: (url) => url.pathname.includes("docs"),
      browser: "Google Chrome",
    },
  ],
};
