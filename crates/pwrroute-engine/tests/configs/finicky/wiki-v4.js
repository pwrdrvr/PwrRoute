// The examples on Finicky's wiki page "Configuration (v4)"
// (github.com/johnste/finicky/wiki), joined into one config: the general
// options, every matching method, every way to select a browser, every
// rewrite method, and the complete example's rules. Hosts are kept apart so
// each link exercises one example.
export default {
  defaultBrowser: "Google Chrome",
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
    { match: "string.example.org/*", url: "https://gitlab.com" },
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
    { match: "apple.com/*", browser: "Safari" },
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
        /^http:\/\/combined.(org|com)\/.*/,
        (url) => url.pathname.includes("/hello"),
      ],
      browser: "Combined",
    },
    // Selecting a browser
    { match: (url) => url.host.endsWith("simple.example.org"), browser: "Firefox" },
    { match: "byname.example/*", browser: "Google Chrome" },
    { match: "bybundle.example/*", browser: "com.google.Chrome" },
    { match: "bypath.example/*", browser: "/Applications/Firefox.app" },
    { match: "profile-string.example/*", browser: "Google Chrome:Personal" },
    { match: "profile-object.example/*", browser: { name: "Google Chrome", profile: "Personal" } },
    {
      match: "advanced.example/*",
      browser: { name: "Google Chrome", appType: "appName", openInBackground: true, profile: "Work" },
    },
    {
      match: "dynamic.example/*",
      browser: (url) => {
        if (url.host === "example.com") return "Google Chrome";
        if (url.host === "work.example.com") return "Firefox";
        return "Safari";
      },
    },
    {
      match: "*facebook*",
      browser: (url) => ({ name: "Google Chrome", openInBackground: url.href.includes("facebook") }),
    },
    // Complete example
    {
      match: "work.example.com/*",
      browser: { name: "Google Chrome", profile: "Work", openInBackground: true },
    },
    { match: "personal.example.com/*", browser: "Google Chrome:Personal" },
    { match: "safari.example.com/*", browser: "Safari" },
    { match: (url) => url.host.endsWith(".dev"), browser: "Firefox" },
  ],
};
