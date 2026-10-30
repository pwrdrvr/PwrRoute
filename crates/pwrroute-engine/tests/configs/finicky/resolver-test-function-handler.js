// TestResolveURL_JSConfigFunctionHandler in Finicky's
// apps/finicky/src/resolver/resolver_test.go.
module.exports = {
  defaultBrowser: "Safari",
  handlers: [
    {
      match: function (request, { opener }) {
        return opener && opener.bundleId === "com.apple.Terminal";
      },
      browser: "Firefox",
    },
  ],
};
