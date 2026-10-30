// Recipes from Finicky's wiki page "Configuration ideas"
// (github.com/johnste/finicky/wiki), with each recipe's code kept and its
// hosts, URL schemes and app names replaced by neutral stand-ins. Many are
// Finicky 3 recipes: `({ url })` and `({ opener })` destructure the URL
// argument, which Finicky 4 still answers through FinickyURL's legacy `url`
// and `opener` getters (with a deprecation warning). Recipes that do not
// validate under Finicky 4 (a rewrite `url` object such as
// `{ host: "smile.shop.example" }`), never parse, or pick a random host are
// left out; tests/finicky_suite.rs covers the first two.
export default {
  defaultBrowser: "Home Browser",
  rewrite: [
    // Force https for all urls
    {
      match: ({ url }) => url.protocol === "http",
      url: (url) => {
        url.protocol = "https";
        return url;
      },
    },
    // MacOS: Create bookmarks that open Finicky
    {
      match: "finicky://open?url=*",
      url: (url) => {
        const target = url.searchParams.get("url");
        return target;
      },
    },
    // Remove all marketing/tracking information from URLs
    {
      match: (m) => true,
      url: (url) => {
        const lower = (s) => String(s || "").toLowerCase();
        const removeExact = new Set(["fbclid", "gclid", "dclid", "gbraid", "wbraid", "msclkid", "ttclid", "twclid", "li_fat_id", "mkt_tok", "mc_cid", "mc_eid", "igsh", "si", "feature", "ref", "ref_src", "spm"]);
        const removePrefixes = ["utm_", "uta_", "ga_", "pk_", "vero_"];
        const removeByValue = new Set(["share", "social", "social_media", "social_network"]);

        const keys = [...url.searchParams.keys()];

        for (const key of keys) {
          const k = lower(key),
            v = lower(url.searchParams.get(key));
          const isExact = removeExact.has(k);
          const isPrefix = removePrefixes.some((p) => k.startsWith(p));
          const isValueNoise = (k === "source" || k === "src" || k === "medium") && removeByValue.has(v);

          if (isExact || isPrefix || isValueNoise) {
            url.searchParams.delete(key);
          }
        }

        return url.href;
      },
    },
    // Redirect a search engine's links to another
    {
      match: finicky.matchDomains(["search.example"]),
      url: "https://private-search.example",
    },
    // Open meeting links in the meeting app, with or without a password
    {
      match: (url) => url.host.includes("meet.example") && url.pathname.includes("/j/"),
      url: (url) => {
        try {
          const match = url.search.match(/pwd=(\w*)/);
          var pass = match ? "&pwd=" + match[1] : "";
        } catch {
          var pass = "";
        }
        const pathMatch = url.pathname.match(/\/j\/(\d+)/);
        var conf = "confno=" + (pathMatch ? pathMatch[1] : "");
        url.search = conf + pass;
        url.pathname = "/join";
        url.protocol = "meetapp";
        return url;
      },
    },
    // Skip a social site's link tracking (away.php)
    {
      match: /social\.example\/away.php/,
      url: ({ url }) => {
        const match = url.search.match(/to=(.+)/);
        return !match ? url : decodeURIComponent(decodeURIComponent(match[1]));
      },
    },
    // Open links in a task tracker's desktop app
    {
      match: "https://app.tasks.example/*",
      url: ({ url }) => ({
        ...url,
        protocol: "tasks",
        host: "",
        pathname: url.pathname.slice(1),
      }),
    },
  ],
  handlers: [
    // Open links in a task tracker's desktop app
    { match: ({ url }) => url.protocol === "tasks", browser: "Tasks App" },
    // Open meeting links in the meeting app
    { match: /meet\.example\/join/, browser: "com.example.meetings" },
    // Matching an array of multiple apps
    {
      match: (_url, { opener }) => ["com.example.mail", "com.example.calendar"].includes(opener?.bundleId ?? ""),
      browser: "Work Browser",
    },
    // Match multiple conditions
    {
      match: ({ opener, url }) => opener.bundleId === "com.example.chat" && url.host.includes("drive.example"),
      browser: "Personal Browser",
    },
    // Open links of example.com including 'Finicky' in the path
    {
      match: ({ url }) => url.host.includes("example.com") && url.pathname.includes("Finicky"),
      browser: "Personal Browser",
    },
    // Open board links in the boards app (a handler's `url` is not Finicky 4
    // API: the handler validates and the key is ignored)
    {
      match: finicky.matchDomains(["boards.example"]),
      url: ({ url }) => ({ ...url, protocol: "boards" }),
      browser: "Boards App",
    },
    // Open music links in the music app
    { match: finicky.matchDomains("open.music.example"), browser: "Music App" },
    // Open team chat links in the native app
    {
      match: finicky.matchHostnames(["teamchat.example"]),
      browser: "com.example.teamchat",
      url: ({ url }) => ({ ...url, protocol: "teamchat" }),
    },
    // Open design links in the design app
    { match: "https://www.design.example/file/*", browser: "Design App" },
    // Open community links in the community app
    { match: "https://community.example/*", url: { protocol: "community" }, browser: "Community App" },
    // Open video meeting links in the video app
    {
      match: ({ url }) => url.host.includes("video.your-selfhosted-server.example") || url.host.includes("video.example"),
      url: ({ url }) => ({ ...url, protocol: "video-meet", host: url.host, pathname: url.pathname }),
      browser: "/Applications/Video\ Meet.app",
    },
    // Open links clicked in a messenger in a specific browser profile
    {
      match: ({ opener }) => ["Messenger"].includes(opener.name),
      browser: { name: "Client Browser", profile: "Profile 1" },
    },
    // Open browser web apps with the correct URL
    {
      match: /^https?:\/\/calls\.example\//,
      browser: (url) => ({
        name: "Work Browser",
        profile: "Default",
        args: ["--app-id=abcdefghijklmnopabcdefghijklmnop", `--app-launch-url-for-shortcuts-menu-item=${url.toString()}`],
      }),
    },
    // Match only one of multiple conditions
    {
      match: (url) => {
        const workDomains = ["office.example", "files.example", "corp.example"];
        return workDomains.some((domain) => url.host.endsWith(domain));
      },
      browser: "Work Browser",
    },
    // Open music store links in the music app
    { match: ["store.music.example*", "geo.store.music.example*"], url: { protocol: "musicstore" }, browser: "Music App" },
  ],
};
