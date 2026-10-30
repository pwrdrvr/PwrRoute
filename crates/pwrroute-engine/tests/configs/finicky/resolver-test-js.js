// TestResolveURL_JSConfig in Finicky's apps/finicky/src/resolver/
// resolver_test.go, with neutral hosts and app names. The Go test assigns the
// object to the namespace without a default export; CommonJS is that legacy
// form.
module.exports = {
  defaultBrowser: "Home Browser",
  handlers: [
    { match: "*code.example/*", browser: "Personal Browser" },
    { match: "https://tracker.example/*", browser: "Work Browser" },
  ],
};
