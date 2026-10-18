// Finicky wildcard patterns at their edges. Each handler names a different
// browser, so the result says which pattern matched first.
import type { FinickyConfig } from "/Applications/Finicky.app/Contents/Resources/finicky.d.ts";

export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: "exact.example", browser: "Exact Bare" },
    { match: "https://exact.example/", browser: "Exact Full" },
    { match: "https://*.sub.example/*", browser: "Sub Https" },
    { match: "*.sub.example/*", browser: "Sub Any Scheme" },
    { match: "http*://scheme.example/*", browser: "Scheme Glob" },
    { match: "path.example/docs*", browser: "Path Prefix" },
    { match: "*?share=*", browser: "Query Anywhere" },
    { match: "port.example:8443/*", browser: "With Port" },
    { match: "literal.example/a\\*b", browser: "Escaped Star" },
    { match: "xn--bcher-kva.example/*", browser: "Punycode" },
    { match: "mailto:*@team.example", browser: "Mail" },
    { match: "*/case/*", browser: "Path Anywhere" },
    { match: "", browser: "Never" },
    { match: "*", browser: "Everything Else" },
  ],
} satisfies FinickyConfig;
