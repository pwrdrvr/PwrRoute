// The Finicky importer's fixture: one rule of every kind it reads, and the
// helper-function matchers a hand-written config grows. Every host is
// reserved for examples.
const hostnameIs = (url: URL, host: string): boolean => url.hostname === host;

function githubOrg(url: URL, org: string): boolean {
  return url.hostname === "github.com" && url.pathname.split("/")[1] === org;
}

export default {
  defaultBrowser: { name: "Google Chrome", profile: "Personal" },
  rewrite: [
    {
      match: "short.example/*",
      url: "https://long.example/",
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
      match: "plain.example/*",
      browser: "Firefox",
    },
    {
      match: ["one.example/*", "*.one.example/*"],
      browser: "Firefox:work",
    },
    {
      match: /^https:\/\/news\.example\.com/,
      browser: "Safari",
    },
    {
      match: (url: URL) => hostnameIs(url, "alpha.example"),
      browser: { name: "Google Chrome", profile: "Work" },
    },
    {
      match: (url: URL) => githubOrg(url, "acme"),
      browser: { name: "Google Chrome", profile: "Work" },
    },
    {
      match: (url: URL) => url.hostname.toLowerCase() === "beta.example",
      browser: { name: "Firefox", profile: "default-release" },
    },
    {
      match: (url: URL) => url.hostname.endsWith("gamma.example"),
      browser: "Firefox",
    },
    {
      match: (url: URL) => hostnameIs(url, "delta.example") && !url.pathname.startsWith("/admin"),
      browser: "Firefox",
    },
    {
      match: (url: URL, { opener }: { opener: { name: string } | null }) =>
        opener?.name === "Slack",
      browser: "Firefox",
    },
    {
      match: "pick.example/*",
      browser: (url: URL) => (url.pathname.startsWith("/a") ? "Safari" : "Firefox"),
    },
    {
      match: "args.example/*",
      browser: { name: "Firefox", args: ["--new-window"] },
    },
  ],
};
