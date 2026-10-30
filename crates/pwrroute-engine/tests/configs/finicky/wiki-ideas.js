// Recipes from Finicky's wiki page "Configuration ideas"
// (github.com/johnste/finicky/wiki), as written there. Many are Finicky 3
// recipes: `({ url })` and `({ opener })` destructure the URL argument, which
// Finicky 4 still answers through FinickyURL's legacy `url` and `opener`
// getters (with a deprecation warning). Recipes that do not validate under
// Finicky 4 (a rewrite `url` object such as `{ host: "smile.amazon.com" }`)
// or never parse are left out; tests/finicky_suite.rs covers those.
export default {
  defaultBrowser: "Safari",
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
    // Redirect google links to duckduckgo.com
    {
      match: finicky.matchDomains(["google.com"]),
      url: "https://duckduckgo.com",
    },
    // Open Zoom links in Zoom app with or without password
    {
      match: (url) => url.host.includes("zoom.us") && url.pathname.includes("/j/"),
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
        url.protocol = "zoommtg";
        return url;
      },
    },
    // Skip vk.com link tracking (vk.com/away.php)
    {
      match: /vk\.com\/away.php/,
      url: ({ url }) => {
        const match = url.search.match(/to=(.+)/);
        return !match ? url : decodeURIComponent(decodeURIComponent(match[1]));
      },
    },
    // Open links in the ClickUp desktop app
    {
      match: "https://app.clickup.com/*",
      url: ({ url }) => ({
        ...url,
        protocol: "clickup",
        host: "",
        pathname: url.pathname.slice(1),
      }),
    },
  ],
  handlers: [
    // Open links in the ClickUp desktop app
    { match: ({ url }) => url.protocol === "clickup", browser: "ClickUp" },
    // Open Zoom links in Zoom app
    { match: /zoom\.us\/join/, browser: "us.zoom.xos" },
    // Matching an array of multiple apps
    {
      match: (_url, { opener }) => ["com.apple.mail", "com.microsoft.Outlook"].includes(opener?.bundleId ?? ""),
      browser: "Google Chrome",
    },
    // Match multiple conditions
    {
      match: ({ opener, url }) => opener.bundleId === "com.tinyspeck.slackmacgap" && url.host.includes("drive.google.com"),
      browser: "Firefox",
    },
    // Open links of example.com including 'Finicky' in the path
    {
      match: ({ url }) => url.host.includes("example.com") && url.pathname.includes("Finicky"),
      browser: "Firefox",
    },
    // Open Trello links in Trello app (a handler's `url` is not Finicky 4
    // API: the handler validates and the key is ignored)
    {
      match: finicky.matchDomains(["trello.com"]),
      url: ({ url }) => ({ ...url, protocol: "trello" }),
      browser: "Trello",
    },
    // Open Spotify links in Spotify app
    { match: finicky.matchDomains("open.spotify.com"), browser: "Spotify" },
    // Open Microsoft Teams links in the native app
    {
      match: finicky.matchHostnames(["teams.microsoft.com"]),
      browser: "com.microsoft.teams",
      url: ({ url }) => ({ ...url, protocol: "msteams" }),
    },
    // Open Figma links in Figma app
    { match: "https://www.figma.com/file/*", browser: "Figma" },
    // Open Discord links in Discord app
    { match: "https://discord.com/*", url: { protocol: "discord" }, browser: "Discord" },
    // Open Jitsi links in Jitsi desktop app
    {
      match: ({ url }) => url.host.includes("jitsi.your-selfhosted-server.com") || url.host.includes("meet.jit.si"),
      url: ({ url }) => ({ ...url, protocol: "jitsi-meet", host: url.host, pathname: url.pathname }),
      browser: "/Applications/Jitsi\ Meet.app",
    },
    // Open links clicked in Telegram in specific Profile in Edge
    {
      match: ({ opener }) => ["Telegram"].includes(opener.name),
      browser: { name: "Microsoft Edge", profile: "Profile 1" },
    },
    // Open Chrome PWA apps with the correct URL
    {
      match: /^https?:\/\/meet\.google\.com\//,
      browser: (url) => ({
        name: "Google Chrome",
        profile: "Default",
        args: ["--app-id=kjgfgldnnfoeklkmfkjfagphfepbbdan", `--app-launch-url-for-shortcuts-menu-item=${url.toString()}`],
      }),
    },
    // Match only one of multiple conditions
    {
      match: (url) => {
        const chromeDomains = ["office.com", "box.com", "microsoft.com"];
        return chromeDomains.some((domain) => url.host.endsWith(domain));
      },
      browser: "Google Chrome",
    },
    // Open Apple Music links in the Music app
    { match: ["music.apple.com*", "geo.music.apple.com*"], url: { protocol: "itmss" }, browser: "Music" },
  ],
};
