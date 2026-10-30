// handlerConfig from Finicky's packages/config-api/src/config.test.ts, with
// neutral hosts and app names.
export default {
  defaultBrowser: "Personal Browser",
  handlers: [
    { match: "player.music.example*", browser: "Music App" },
    { match: ["code.example*", /shop/], browser: "Work Browser" },
    {
      match: ["design.example*", "draw.example*"],
      browser: { name: "Work Browser", profile: "Design" },
    },
    {
      match: (url) => url.pathname.includes("docs"),
      browser: "Work Browser",
    },
  ],
};
