// edgeCaseConfig from Finicky's packages/config-api/src/config.test.ts, with
// neutral app names.
export default {
  defaultBrowser: "Home Browser",
  handlers: [
    { match: "", browser: "Browser A" },
    { match: "https://*", browser: "Personal Browser" },
    { match: "*?param=value*", browser: "Browser B" },
  ],
};
