// Handlers that read the opener the ways configs do.
export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: (url, { opener }) => opener?.bundleId === "com.tinyspeck.slackmacgap", browser: { name: "Google Chrome", profile: "Work" } },
    { match: (url, { opener }) => !!opener && opener.name === "Terminal" && url.hostname === "localhost", browser: "Firefox Developer Edition" },
    { match: (url, { opener }) => !!opener && opener.path.startsWith("/Applications/Microsoft Outlook.app"), browser: "Microsoft Edge" },
    { match: (url, { opener }) => opener === null && url.hostname === "direct.example", browser: "Arc" },
    { match: (url, { opener }) => !!opener && !!opener.windowTitle, browser: "Window Title Seen" },
  ],
};
