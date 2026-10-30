// TestResolveURL_JSConfigFunctionHandler in Finicky's
// apps/finicky/src/resolver/resolver_test.go, with neutral app names.
module.exports = {
  defaultBrowser: "Home Browser",
  handlers: [
    {
      match: function (request, { opener }) {
        return opener && opener.bundleId === "com.example.terminal";
      },
      browser: "Personal Browser",
    },
  ],
};
