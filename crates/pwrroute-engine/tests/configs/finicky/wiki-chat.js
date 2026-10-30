// The wiki's recipe for opening a team chat's web links in its desktop app
// (Finicky's wiki page "Configuration ideas", github.com/johnste/finicky/wiki),
// with its code kept and the chat service's hosts and scheme replaced by
// neutral stand-ins (`*.chat.example`, `chat://`, so the host slice is 13
// characters). One workspace and one enterprise grid fill its mappings. A
// CommonJS config whose callbacks use Finicky 3's `({ url })` argument.
// Team IDs start with a T (E for an enterprise grid) in the web client's URL,
// https://app.chat.example/client/TXXXXXXX/CXXXXXXX.
const workChatTeamMapping = {
  // 'subdomain': 'TXXXXXXX',
  acme: "T0ACME",
};

const personalChatMapping = {
  // personal workspaces
  "acme.enterprise": "E0ACME",
};

const chatSubdomainMapping = {
  ...workChatTeamMapping,
  ...personalChatMapping,
};

const chatRewriter = {
  match: ["*.chat.example/*"],
  url: function (urlObj) {
    const subdomain = urlObj.host.slice(0, -13); // before .chat.example
    const pathParts = urlObj.pathname.split("/");

    let team,
      patterns = {};
    if (subdomain != "app") {
      if (!Object.keys(chatSubdomainMapping).includes(subdomain)) {
        console.log(
          `No team ID found for ${urlObj.host}`,
          `Add a correct team ID to ~/.finicky.js to allow direct linking to the chat app.`
        );
        return urlObj;
      }
      team = chatSubdomainMapping[subdomain];

      if (subdomain.slice(-11) == ".enterprise") {
        patterns = {
          file: [/\/files\/\w+\/(?<id>\w+)/],
        };
      } else {
        patterns = {
          file: [/\/messages\/\w+\/files\/(?<id>\w+)/],
          team: [/(?:\/messages\/\w+)?\/team\/(?<id>\w+)/],
          channel: [
            /\/(?:messages|archives)\/(?<id>\w+)(?:\/(?<message>p\d+))?/,
          ],
        };
      }
    } else {
      patterns = {
        file: [
          /\/client\/(?<team>\w+)\/\w+\/files\/(?<id>\w+)/,
          /\/docs\/(?<team>\w+)\/(?<id>\w+)/,
        ],
        team: [/\/client\/(?<team>\w+)\/\w+\/user_profile\/(?<id>\w+)/],
        channel: [
          /\/client\/(?<team>\w+)\/(?<id>\w+)(?:\/(?<message>[\d.]+))?/,
        ],
      };
    }

    for (let [host, host_patterns] of Object.entries(patterns)) {
      for (let pattern of host_patterns) {
        let match = pattern.exec(urlObj.pathname);
        if (match) {
          let search = `team=${team || match.groups.team}`;

          if (match.groups.id) {
            search += `&id=${match.groups.id}`;
          }

          if (match.groups.message) {
            let message = match.groups.message;
            if (message.charAt(0) == "p") {
              message = message.slice(1, 11) + "." + message.slice(11);
            }
            search += `&message=${message}`;
          }

          let outputStr = `chat://${host}?${search}`;
          console.log(
            `Rewrote chat URL ${urlObj.href} to deep link ${outputStr}`
          );
          
          return new URL(outputStr);
        }
      }
    }

    return urlObj;
  },
};

module.exports = {
  defaultBrowser: "Personal Browser",
  rewrite: [chatRewriter],
  handlers: [
    {
      match: ({ url }) => {
        // Check for both 'chat:' and 'chat' since the property might not include the colon
        return url.protocol === "chat:" || url.protocol === "chat";
      },
      browser: "Chat App",
    },
    {
      // Optional. If these work url cannot be converted, open them is work browser
      // Login in work workspace unfortunately lands on the personal browser, just copy the link to the work browser
      match: ({ url }) => {
        const workDomains = Object.keys(workChatTeamMapping).map(subdomain => subdomain + ".chat.example");
        return workDomains.includes(url.host);
      },
      browser: "Work Browser", // your work browser
    },
  ],
};
