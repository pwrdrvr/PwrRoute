// wildcardConfig from Finicky's packages/config-api/src/config.test.ts, with
// neutral hosts and app names.
export default {
  defaultBrowser: "Home Browser",
  handlers: [
    { match: "*.example.com*", browser: "Browser A" },
    { match: ["*.search.*", "mail.*.example*"], browser: "Browser A" },
  ],
};
