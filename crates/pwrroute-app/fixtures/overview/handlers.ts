// A Finicky config of inline matchers, for the Overview's wide map (1e)
// in the screens example: contrived, with example.com, .org and .net hosts
// only. Finicky names each inline arrow "match" after its property, so the
// rows read each function's source; the last handler is a
// finicky.matchHostnames list, which the map shows as its hostnames. All
// of them open one Edge profile.
const team = { name: "Microsoft Edge", profile: "Research and Development Team" };

export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: ({ url }) => url.host === "tickets.example.com", browser: team },
    {
      match: (url) => url.host.endsWith(".build.example.net") && url.pathname.startsWith("/runs/"),
      browser: team,
    },
    {
      match: (url) => {
        if (url.host === "wiki.example.org") {
          return true;
        }
        return false;
      },
      browser: team,
    },
    { match: (url) => /^(docs|api)\.example\.com$/.test(url.host), browser: team },
    { match: (url) => url.searchParams.get("team") === "research", browser: team },
    {
      match: function (url) {
        return url.host === "review.example.com" && url.pathname.includes("/pull/");
      },
      browser: team,
    },
    { match: (url) => ["chat.example.com", "meet.example.com"].includes(url.host), browser: team },
    {
      match: finicky.matchHostnames(["status.example.com", "metrics.example.net", /^dash\d+\.example\.org$/]),
      browser: team,
    },
  ],
};
