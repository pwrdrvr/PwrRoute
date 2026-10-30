// TestResolveURL_RewriteRule in Finicky's
// apps/finicky/src/resolver/resolver_test.go, with neutral hosts.
module.exports = {
  defaultBrowser: "Home Browser",
  rewrite: [
    {
      match: "https://www.video.example/watch*",
      url: function (url) {
        return url.href.replace("https://www.video.example/watch", "https://vid.example/");
      },
    },
  ],
};
