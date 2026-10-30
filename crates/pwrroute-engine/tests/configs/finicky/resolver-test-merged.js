// TestResolveURL_MergedJSAndJSON(_WithCachedRules) in Finicky's
// apps/finicky/src/resolver/resolver_test.go, with neutral hosts and app names.
module.exports = {
  defaultBrowser: "Home Browser",
  handlers: [{ match: "*code.example/*", browser: "Personal Browser" }],
};
