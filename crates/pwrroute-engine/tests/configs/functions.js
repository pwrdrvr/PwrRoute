// Function matchers and browsers, chained and legacy rewrites, originalUrl,
// and the error paths: a matcher that throws, a browser function that returns
// nothing, an invalid browser object.
export default {
  defaultBrowser: (url) => (url.hostname.endsWith(".dev") ? "Firefox Developer Edition" : "Safari"),
  rewrite: [
    {
      match: "*://chain.example/*",
      url: (url) => {
        url.pathname = url.pathname + "/one";
        return url;
      },
    },
    { match: "*://chain.example/*/one", url: (url) => url.href + "?two=2" },
    {
      match: "legacy.example/*",
      url: () => ({ username: "", protocol: "https", host: "modern.example", pathname: "/from-legacy", search: "a=1" }),
    },
    {
      match: (url) => {
        if (url.hostname === "boom.example") throw new Error("rewrite matcher threw");
        return false;
      },
      url: (url) => url,
    },
    { match: "https://static.example/*", url: "https://static-target.example/landing" },
  ],
  handlers: [
    {
      match: (url, { originalUrl }) => !!originalUrl && originalUrl.hostname === "chain.example",
      browser: { name: "Firefox", profile: "chain" },
    },
    { match: /^https:\/\/CASE\.example\//i, browser: "Google Chrome:Profile 2" },
    { match: "https://bundle.example/*", browser: "com.google.Chrome" },
    { match: "https://path.example/*", browser: "/Applications/Firefox.app" },
    { match: "https://undefined.example/*", browser: () => undefined },
    { match: "https://invalid.example/*", browser: () => ({ name: 42 }) },
    {
      match: "https://args.example/*",
      browser: (url) => ({
        name: "Google Chrome",
        args: ["--incognito", url.searchParams.get("q") || ""],
        openInBackground: true,
      }),
    },
    { match: finicky.matchHostnames([/^api\./, "exact.example"]), browser: "Arc" },
    { match: ["https://first.example/*", "https://second.example/*"], browser: "Brave Browser" },
    {
      match: (url) => {
        if (url.hostname === "handler-boom.example") throw new Error("handler matcher threw");
        return false;
      },
      browser: "Opera",
    },
    { match: "https://modern.example/*", browser: { name: "Safari", openInBackground: false } },
    { match: "https://static-target.example/*", browser: "Microsoft Edge" },
  ],
};
