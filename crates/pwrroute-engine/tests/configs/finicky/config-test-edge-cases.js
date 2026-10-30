// edgeCaseConfig from Finicky's packages/config-api/src/config.test.ts.
export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: "", browser: "Chrome" },
    { match: "https://*", browser: "Firefox" },
    { match: "*?param=value*", browser: "Edge" },
  ],
};
