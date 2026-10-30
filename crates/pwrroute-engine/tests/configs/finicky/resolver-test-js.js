// TestResolveURL_JSConfig and TestResolveURL_MergedJSAndJSON in Finicky's
// apps/finicky/src/resolver/resolver_test.go. The Go test assigns the object
// to the namespace without a default export; CommonJS is that legacy form.
module.exports = {
  defaultBrowser: "Safari",
  handlers: [
    { match: "*github.com/*", browser: "Firefox" },
    { match: "https://linear.app/*", browser: "Google Chrome" },
  ],
};
