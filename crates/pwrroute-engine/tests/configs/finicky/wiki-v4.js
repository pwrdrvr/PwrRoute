// The examples on Finicky's wiki page "Configuration (v4)"
// (github.com/johnste/finicky/wiki), joined into one config: the general
// options, every matching method, every way to select a browser, every
// rewrite method, and the complete example's rules. Hosts are kept apart so
// each link exercises one example. Hosts, top-level domains and app names
// are neutral stand-ins for the wiki's.
export default {
  defaultBrowser: "Work Browser",
  options: {
    checkForUpdates: true,
    logRequests: false,
    keepRunning: false,
    hideIcon: false,
  },
  rewrite: [
    // General options
    { match: "example.org/*", url: "http://example.com" },
    // Rewriting urls: simple string replacement, URL instance, dynamic
    // modification, complex transformation
    { match: "string.example.org/*", url: "https://code.example" },
    { match: "instance.example.org/*", url: new URL("https://example.com") },
    {
      match: "dynamic.example.org/*",
      url: (url) => {
        url.pathname = "/hello";
        return url;
      },
    },
    {
      match: /^https?:\/\/old\.example\.com\/(.*)/,
      url: (url) => {
        const newUrl = new URL("https://new.example.com");
        newUrl.pathname = url.pathname;
        newUrl.search = url.search;
        return newUrl;
      },
    },
    // Complete example
    { match: "old.example.net/*", url: "https://new.example.net" },
    {
      match: /^https?:\/\/redirect\.example\.com\/(.*)/,
      url: (url) => {
        const newUrl = new URL("https://target.example.com");
        newUrl.pathname = url.pathname;
        return newUrl;
      },
    },
  ],
  handlers: [
    // General options
    { match: "vendor.example/*", browser: "Home Browser" },
    // Matching urls
    { match: "http://match.example.com/*", browser: "String Wildcard" },
    { match: "*.sub.example.com/*", browser: "Subdomain Wildcard" },
    { match: /^https?:\/\/regex\.example\.com\/.*$/, browser: "Regex" },
    { match: /\.(dev|test)\.example\.com$/, browser: "Regex Subdomain" },
    { match: (url) => url.host === "fn.example.com", browser: "Function Host" },
    { match: (url) => url.pathname.startsWith("/api/"), browser: "Function Path" },
    {
      match: [
        "https://combined.example.com",
        /^http:\/\/example.(org|com)\/.*/,
        (url) => url.pathname.includes("/hello"),
      ],
      browser: "Combined",
    },
    // Selecting a browser
    { match: (url) => url.host.endsWith("simple.example.org"), browser: "Personal Browser" },
    { match: "byname.example/*", browser: "Work Browser" },
    { match: "bybundle.example/*", browser: "com.example.WorkBrowser" },
    { match: "bypath.example/*", browser: "/Applications/Personal Browser.app" },
    { match: "profile-string.example/*", browser: "Work Browser:Personal" },
    { match: "profile-object.example/*", browser: { name: "Work Browser", profile: "Personal" } },
    {
      match: "advanced.example/*",
      browser: { name: "Work Browser", appType: "appName", openInBackground: true, profile: "Work" },
    },
    {
      match: "dynamic.example/*",
      browser: (url) => {
        if (url.host === "example.com") return "Work Browser";
        if (url.host === "work.example.com") return "Personal Browser";
        return "Home Browser";
      },
    },
    {
      match: "*social*",
      browser: (url) => ({ name: "Work Browser", openInBackground: url.href.includes("social") }),
    },
    // Complete example
    {
      match: "work.example.com/*",
      browser: { name: "Work Browser", profile: "Work", openInBackground: true },
    },
    { match: "personal.example.com/*", browser: "Work Browser:Personal" },
    { match: "home.example.com/*", browser: "Home Browser" },
    { match: (url) => url.host.endsWith(".test"), browser: "Personal Browser" },
  ],
};
