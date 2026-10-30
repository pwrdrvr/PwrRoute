/** @type {Parameters<typeof import("finicky").default>[0]} */
module.exports = {
  defaultBrowser: "Home Browser",
  handlers: [
    // This rule lives in the JS config and takes priority over rules.json
    {
      match: "code.example/*",
      browser: "Work Browser",
    },
    {
      match: /tracker\.example/,
      browser: "Personal Browser",
    },
  ],
  options: {
    keepRunning: true,
    logRequests: true,
    checkForUpdates: true,
  },
};
