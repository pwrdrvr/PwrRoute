// The Finicky config of the Overview design (2a-2b, 3c): contrived, for the
// app's tests and screenshots. Hosts are example.com, .org and .net only.
// Two rewrites and five handlers: a named function matcher, two wildcards
// to a Chrome profile, a function that picks the browser, and a browser
// that is not installed. Its defaultBrowser, Safari, takes everything else
// until PwrRoute's default is asked to.
export default {
  defaultBrowser: "Safari",
  rewrite: [
    {
      match: "http://*.example.com/*",
      url: (url: URL) => {
        url.protocol = "https:";
        return url;
      },
    },
    {
      match: "*",
      url: (url: URL) => {
        url.searchParams.delete("utm_source");
        return url;
      },
    },
  ],
  handlers: [
    {
      match: function isWorkplace(url: URL) {
        return url.host.endsWith("workplace.example.com");
      },
      browser: "Firefox",
    },
    {
      match: "chat.example.com/*",
      browser: { name: "Google Chrome", profile: "Work" },
    },
    {
      match: ["*.docs.example.com/*", "docs.example.org/*"],
      browser: { name: "Google Chrome", profile: "Work" },
    },
    {
      match: "meet.example.com/*",
      browser: (url: URL) => (url.pathname.startsWith("/j/") ? "Firefox" : "Safari"),
    },
    {
      match: "beta.example.net/*",
      browser: "Orion",
    },
  ],
};
