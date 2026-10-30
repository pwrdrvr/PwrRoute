// TestResolveURL_RewriteRule in Finicky's
// apps/finicky/src/resolver/resolver_test.go.
module.exports = {
  defaultBrowser: "Safari",
  rewrite: [
    {
      match: "https://www.youtube.com/watch*",
      url: function (url) {
        return url.href.replace("https://www.youtube.com/watch", "https://youtu.be/");
      },
    },
  ],
};
