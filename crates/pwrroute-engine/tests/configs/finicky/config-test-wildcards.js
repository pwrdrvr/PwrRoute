// wildcardConfig from Finicky's packages/config-api/src/config.test.ts.
export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: "*.example.com*", browser: "Chrome" },
    { match: ["*.google.*", "mail.*.com*"], browser: "Chrome" },
  ],
};
