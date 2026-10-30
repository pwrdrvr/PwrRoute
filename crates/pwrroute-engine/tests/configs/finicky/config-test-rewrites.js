// rewriteConfig from Finicky's packages/config-api/src/config.test.ts, with
// neutral hosts.
export default {
  defaultBrowser: "Home Browser",
  rewrite: [
    {
      match: "https://redirect.example/link?url=*",
      url: (url) => url.href,
    },
    {
      match: /^https:\/\/s\.example\/.+/,
      url: (url) => url.href.replace("s.example", "social.example"),
    },
    {
      match: (url) => url.href.includes("shortened"),
      url: (url) => url.href + "/expanded",
    },
    {
      match: (url) => url.href.includes("use-url-object"),
      url: (url) => new URL("https://example.com"),
    },
  ],
};
