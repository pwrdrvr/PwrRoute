// rewriteConfig from Finicky's packages/config-api/src/config.test.ts.
export default {
  defaultBrowser: "Safari",
  rewrite: [
    {
      match: "https://slack-redir.net/link?url=*",
      url: (url) => url.href,
    },
    {
      match: /^https:\/\/t\.co\/.+/,
      url: (url) => url.href.replace("t.co", "twitter.com"),
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
