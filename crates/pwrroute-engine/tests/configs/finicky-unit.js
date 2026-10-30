// Finicky's own unit tests for its configuration API
// (packages/config-api/src/*.test.ts in johnste/finicky), ported case by case.
// Not a config: a plain script evaluated after the API, in PwrRoute's engine
// (tests/finicky_suite.rs) and in the Go host's goja VM
// (tests/oracle/evaloracle.go), so both hosts run the same cases.
//
// Each `it` keeps Finicky's describe/it names and its expectations. vitest's
// `vi.spyOn(console, "warn")` becomes a swap of `console.warn` for the case.
// The API does not export FinickyURL or matchWildcard; a FinickyURL comes from
// `rewriteUrl(string)`, which constructs one, and `matchWildcard(p, s)` is
// what `isMatch(p, url)` calls with `url.href`, so it is reached with
// `{ href: s }`. Where vitest mocked a module function, the comment says what
// the port asserts instead.

var finickyUnit = (function () {
  var api = finickyConfigAPI;
  var cases = [];
  var file = "";
  var path = [];

  function describe(name, body) {
    path.push(name);
    body();
    path.pop();
  }

  function it(name, body) {
    cases.push({ file: file, name: path.concat([name]).join(" > "), body: body });
  }

  function show(value) {
    try {
      if (typeof value === "function") return "[function]";
      if (value instanceof RegExp) return String(value);
      var text = JSON.stringify(value);
      return text === undefined ? String(value) : text;
    } catch (e) {
      return String(value);
    }
  }

  function fail(message) {
    throw new Error("ASSERT " + message);
  }

  function deepEqual(a, b) {
    if (a === b) return true;
    if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    var ka = Object.keys(a);
    var kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    for (var i = 0; i < ka.length; i++) {
      if (!deepEqual(a[ka[i]], b[ka[i]])) return false;
    }
    return true;
  }

  // vitest's toMatchObject: every key of `expected` matches, recursively.
  function matchObject(actual, expected) {
    if (expected === null || typeof expected !== "object") return actual === expected;
    if (actual === null || typeof actual !== "object") return false;
    for (var key in expected) {
      if (!matchObject(actual[key], expected[key])) return false;
    }
    return true;
  }

  function expect(actual) {
    return {
      toBe: function (want) {
        if (actual !== want) fail("expected " + show(actual) + " to be " + show(want));
      },
      toEqual: function (want) {
        if (!deepEqual(actual, want)) fail("expected " + show(actual) + " to equal " + show(want));
      },
      toBeNull: function () {
        if (actual !== null) fail("expected " + show(actual) + " to be null");
      },
      toMatchObject: function (want) {
        if (!matchObject(actual, want)) fail("expected " + show(actual) + " to match " + show(want));
      },
      toBeInstanceOf: function (type) {
        if (!(actual instanceof type)) fail("expected " + show(actual) + " to be an instance of " + type.name);
      },
      toBeFinickyURL: function () {
        // vitest: toBeInstanceOf(FinickyURL). The class is not exported, so
        // compare prototypes with one rewriteUrl made.
        if (!(actual instanceof URL) || Object.getPrototypeOf(actual) !== Object.getPrototypeOf(finickyURL("https://a.example/"))) {
          fail("expected " + show(actual) + " to be a FinickyURL");
        }
      },
      toThrow: function (substring) {
        var threw = false;
        var message = "";
        try {
          actual();
        } catch (e) {
          threw = true;
          message = e && e.message !== undefined ? String(e.message) : String(e);
        }
        if (!threw) fail("expected a throw" + (substring ? " containing " + show(substring) : ""));
        if (substring && message.indexOf(substring) === -1) {
          fail("expected the error to contain " + show(substring) + ", got " + show(message));
        }
      },
      notToThrow: function () {
        try {
          actual();
        } catch (e) {
          fail("expected no throw, got " + show(String(e && e.message)));
        }
      },
    };
  }

  // console.warn, spied on for one case: vitest's vi.spyOn(console, "warn").
  function spyWarn(body) {
    var calls = [];
    var original = console.warn;
    console.warn = function () {
      calls.push(Array.prototype.slice.call(arguments).map(String).join(" "));
    };
    try {
      body(calls);
    } finally {
      console.warn = original;
    }
  }

  function warned(calls, substring) {
    for (var i = 0; i < calls.length; i++) {
      if (calls[i].indexOf(substring) !== -1) return;
    }
    fail("expected console.warn with " + show(substring) + ", got " + show(calls));
  }

  function finickyURL(href, opener) {
    return api.rewriteUrl(href, null, { opener: opener === undefined ? null : opener });
  }

  function matchWildcard(pattern, str) {
    return api.isMatch(pattern, { href: str }, { opener: null });
  }

  var mockProcessInfo = { name: "TestApp", bundleId: "com.test.app", path: "/Applications/TestApp.app" };

  // ---- config.test.ts ----------------------------------------------------
  file = "config.test.ts";
  describe("openUrl", function () {
    it("validation", function () {
      expect(function () {
        api.openUrl("https://example.com", mockProcessInfo, null, { defaultBrowser: 123 });
      }).toThrow("Invalid config");
    });

    describe("handlers", function () {
      var handlerConfig = {
        defaultBrowser: "Firefox",
        handlers: [
          { match: "browser.spotify.com*", browser: "Spotify" },
          { match: ["github.com*", /amazon/], browser: "Google Chrome" },
          { match: ["figma.com*", "sketch.com*"], browser: { name: "Google Chrome", profile: "Design" } },
          { match: function (url) { return url.pathname.includes("docs"); }, browser: "Google Chrome" },
        ],
      };
      [
        { url: "https://example.com", expected: "Firefox" },
        { url: "https://browser.spotify.com/track/123", expected: "Spotify" },
        { url: "https://github.com/some-repo", expected: "Google Chrome" },
        { url: "https://amazon.com/product", expected: "Google Chrome" },
        { url: "https://figma.com/file/123", expected: { name: "Google Chrome", profile: "Design" } },
        { url: "https://sketch.com/dashboard", expected: { name: "Google Chrome", profile: "Design" } },
        { url: "https://example.com/docs", expected: "Google Chrome" },
      ].forEach(function (c) {
        it("handles " + c.url, function () {
          var result = api.openUrl(c.url, mockProcessInfo, null, handlerConfig);
          expect(result.browser).toMatchObject(typeof c.expected === "string" ? { name: c.expected } : c.expected);
        });
      });
      it("works with null opener", function () {
        var result = api.openUrl("https://example.com", null, null, handlerConfig);
        expect(result.browser).toMatchObject({ name: "Firefox" });
      });
    });

    describe("rewrites", function () {
      var rewriteConfig = {
        defaultBrowser: "Safari",
        rewrite: [
          { match: "https://slack-redir.net/link?url=*", url: function (url) { return url.href; } },
          { match: /^https:\/\/t\.co\/.+/, url: function (url) { return url.href.replace("t.co", "twitter.com"); } },
          { match: function (url) { return url.href.includes("shortened"); }, url: function (url) { return url.href + "/expanded"; } },
          { match: function (url) { return url.href.includes("use-url-object"); }, url: function () { return new URL("https://example.com"); } },
        ],
      };
      [
        { input: "https://slack-redir.net/link?url=https://example.com", expectedUrl: "https://slack-redir.net/link?url=https://example.com" },
        { input: "https://t.co/abc123", expectedUrl: "https://twitter.com/abc123" },
        { input: "https://my.shortened.url/123", expectedUrl: "https://my.shortened.url/123/expanded" },
        { input: "https://www.use-url-object.com", expectedUrl: "https://example.com/" },
      ].forEach(function (c) {
        it("rewrites " + c.input, function () {
          var result = api.openUrl(c.input, mockProcessInfo, null, rewriteConfig);
          expect(result.browser).toMatchObject({ name: "Safari", url: c.expectedUrl });
        });
      });
    });

    describe("wildcards", function () {
      var wildcardConfig = {
        defaultBrowser: "Safari",
        handlers: [
          { match: "*.example.com*", browser: "Chrome" },
          { match: ["*.google.*", "mail.*.com*"], browser: "Chrome" },
        ],
      };
      [
        { urls: ["https://sub1.example.com", "https://sub2.example.com/path"], expected: "Chrome" },
        { urls: ["https://mail.google.com", "https://docs.google.co.uk", "https://mail.yahoo.com"], expected: "Chrome" },
        { urls: ["https://example.com", "https://google.com"], expected: "Safari" },
      ].forEach(function (c) {
        c.urls.forEach(function (url) {
          it(c.expected + " handles " + url, function () {
            var result = api.openUrl(url, mockProcessInfo, null, wildcardConfig);
            expect(result.browser).toMatchObject({ name: c.expected });
          });
        });
      });
    });

    describe("edge cases", function () {
      var edgeCaseConfig = {
        defaultBrowser: "Safari",
        handlers: [
          { match: "", browser: "Chrome" },
          { match: "https://*", browser: "Firefox" },
          { match: "*?param=value*", browser: "Edge" },
        ],
      };
      [
        { url: "http://example.com", expected: "Safari" },
        { url: "https://example.com", expected: "Firefox" },
        { url: "http://example.com/?param=value&other=123", expected: "Edge" },
      ].forEach(function (c) {
        it(c.expected + " handles " + c.url, function () {
          var result = api.openUrl(c.url, mockProcessInfo, null, edgeCaseConfig);
          expect(result.browser).toMatchObject({ name: c.expected });
        });
      });
    });
  });

  // ---- wildcard.test.ts --------------------------------------------------
  file = "wildcard.test.ts";
  describe("matchWildcard", function () {
    describe("basic wildcard matching", function () {
      it("matches example.com* pattern with various protocols", function () {
        var pattern = "example.com*";
        expect(matchWildcard(pattern, "https://example.com")).toBe(true);
        expect(matchWildcard(pattern, "http://example.com")).toBe(true);
        expect(matchWildcard(pattern, "mailto:example.com")).toBe(true);
        expect(matchWildcard(pattern, "ftp://example.com")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com/path")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com?query=123")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com#fragment")).toBe(true);
      });
      it("does not match non-matching domains", function () {
        var pattern = "example.com*";
        expect(matchWildcard(pattern, "https://notexample.com")).toBe(false);
        expect(matchWildcard(pattern, "https://sub.different.com")).toBe(false);
      });
    });

    describe("wildcard positions", function () {
      it("matches wildcards at the start", function () {
        var pattern = "*.example.com";
        expect(matchWildcard(pattern, "https://sub.example.com")).toBe(true);
        expect(matchWildcard(pattern, "https://another.sub.example.com")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com")).toBe(false);
      });
      it("matches wildcards in the middle", function () {
        var pattern = "example.*/path";
        expect(matchWildcard(pattern, "https://example.com/path")).toBe(true);
        expect(matchWildcard(pattern, "https://example.org/path")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com/other")).toBe(false);
      });
      it("matches multiple wildcards", function () {
        var pattern = "*.example.*/path/*";
        expect(matchWildcard(pattern, "https://sub.example.com/path/test")).toBe(true);
        expect(matchWildcard(pattern, "https://sub.example.org/path/foo/bar")).toBe(true);
        expect(matchWildcard(pattern, "https://sub.example.com/other/test")).toBe(false);
      });
    });

    describe("escaped wildcards", function () {
      it("matches literal asterisks when escaped", function () {
        var pattern = "example\\*.com";
        expect(matchWildcard(pattern, "https://example*.com")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com")).toBe(false);
      });
      it("combines escaped and unescaped wildcards", function () {
        var pattern = "*example\\*.com/*";
        expect(matchWildcard(pattern, "https://test.example*.com/path")).toBe(true);
        expect(matchWildcard(pattern, "https://example*.com/anything")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com/path")).toBe(false);
      });
    });

    describe("special cases", function () {
      it("handles patterns with special regex characters", function () {
        var pattern = "example.com/path?*";
        expect(matchWildcard(pattern, "https://example.com/path?query=123")).toBe(true);
        expect(matchWildcard(pattern, "https://example.com/path")).toBe(false);
      });
      it("handles full URLs with query parameters and fragments", function () {
        var pattern = "*.com/*#*";
        expect(matchWildcard(pattern, "https://example.com/path#fragment")).toBe(true);
        expect(matchWildcard(pattern, "https://sub.example.com/path?query=123#fragment")).toBe(true);
      });
      it("handles invalid patterns gracefully", function () {
        expect(matchWildcard("[invalid.pattern", "https://example.com")).toBe(false);
      });
    });

    describe("additional edge cases", function () {
      it("handles exact matches", function () {
        var pattern = "https://example.com";
        expect(matchWildcard(pattern, "https://example.com")).toBe(true);
        expect(matchWildcard(pattern, "http://example.com")).toBe(false);
      });
      it("handles multiple TLD wildcards", function () {
        var pattern = "*.google.*";
        expect(matchWildcard(pattern, "https://mail.google.com")).toBe(true);
        expect(matchWildcard(pattern, "https://docs.google.co.uk")).toBe(true);
        expect(matchWildcard(pattern, "https://google.com")).toBe(false);
      });
      it("handles complex subdomain patterns", function () {
        var pattern = "https://*.github.io/*";
        expect(matchWildcard(pattern, "https://user1.github.io/repo")).toBe(true);
        expect(matchWildcard(pattern, "https://org.github.io/docs/api")).toBe(true);
        expect(matchWildcard(pattern, "https://github.io/test")).toBe(false);
      });
    });
  });

  // ---- FinickyURL.test.ts ------------------------------------------------
  file = "FinickyURL.test.ts";
  describe("FinickyURL", function () {
    describe("constructor", function () {
      it("should create a FinickyURL instance with a valid URL", function () {
        var url = finickyURL("https://example.com");
        expect(url).toBeFinickyURL();
        expect(url).toBeInstanceOf(URL);
        expect(url.href).toBe("https://example.com/");
      });
      it("should create a FinickyURL instance with opener information", function () {
        var url = finickyURL("https://example.com", mockProcessInfo);
        expect(url).toBeFinickyURL();
        expect(url.href).toBe("https://example.com/");
      });
      it("should throw an error for invalid URLs", function () {
        expect(function () { finickyURL("invalid-url"); }).toThrow();
      });
    });

    describe("urlString property", function () {
      it("should return the href and show deprecation warning", function () {
        spyWarn(function (calls) {
          var url = finickyURL("https://example.com/path?query=value#fragment");
          expect(url.urlString).toBe("https://example.com/path?query=value#fragment");
          warned(calls, 'Accessing legacy property "urlString"');
        });
      });
    });

    describe("url property", function () {
      // vitest also spies on URLtoLegacyURLObject, which is not exported;
      // the object it returns is what is checked here.
      it("should return a LegacyURLObject and show deprecation warning", function () {
        spyWarn(function (calls) {
          var url = finickyURL("https://user:pass@example.com:8080/path?query=value#fragment");
          var legacyUrl = url.url;
          warned(calls, 'Accessing legacy property "url"');
          expect(legacyUrl.host).toBe("example.com");
          expect(legacyUrl.protocol).toBe("https");
          expect(legacyUrl.pathname).toBe("/path");
          expect(legacyUrl.search).toBe("query=value");
          expect(legacyUrl.username).toBe("user");
          expect(legacyUrl.password).toBe("pass");
          expect(legacyUrl.port).toBe(8080);
          expect(legacyUrl.hash).toBe("fragment");
        });
      });
    });

    describe("opener property", function () {
      it("should return the opener and show deprecation warning", function () {
        spyWarn(function (calls) {
          var url = finickyURL("https://example.com", mockProcessInfo);
          expect(url.opener).toEqual(mockProcessInfo);
          warned(calls, 'Accessing legacy property "opener"');
        });
      });
      it("should return null when no opener is provided", function () {
        spyWarn(function (calls) {
          var url = finickyURL("https://example.com");
          expect(url.opener).toBeNull();
          warned(calls, 'Accessing legacy property "opener"');
        });
      });
    });

    describe("keys property", function () {
      it("should throw an error with a message about using finicky.getModifierKeys()", function () {
        var url = finickyURL("https://example.com");
        expect(function () { return url.keys; }).toThrow(
          'Accessing legacy property "keys" that is no longer supported, please use finicky.getModifierKeys() instead.'
        );
      });
    });

    describe("URL inheritance", function () {
      it("should inherit all URL properties", function () {
        var url = finickyURL("https://user:pass@example.com:8080/path?query=value#fragment");
        expect(url.href).toBe("https://user:pass@example.com:8080/path?query=value#fragment");
        expect(url.protocol).toBe("https:");
        expect(url.hostname).toBe("example.com");
        expect(url.host).toBe("example.com:8080");
        expect(url.pathname).toBe("/path");
        expect(url.search).toBe("?query=value");
        expect(url.hash).toBe("#fragment");
        expect(url.username).toBe("user");
        expect(url.password).toBe("pass");
        expect(url.port).toBe("8080");
        expect(url.origin).toBe("https://example.com:8080");
      });
      it("should inherit URL methods", function () {
        var url = finickyURL("https://example.com/path");
        url.searchParams.append("query", "value");
        expect(url.href).toBe("https://example.com/path?query=value");
        url.pathname = "/newpath";
        expect(url.href).toBe("https://example.com/newpath?query=value");
      });
    });
  });

  // ---- utils.test.ts -----------------------------------------------------
  file = "utils.test.ts";
  var mockOptions = { opener: mockProcessInfo };

  describe("isMatch", function () {
    var mockUrl = finickyURL("https://example.com/path?query=value#fragment");
    describe("string patterns", function () {
      it("matches exact strings", function () {
        expect(api.isMatch("https://example.com/path?query=value#fragment", mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch("https://different.com", mockUrl, mockOptions)).toBe(false);
      });
      it("matches wildcard patterns", function () {
        expect(api.isMatch("example.com*", mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch("*.com/path*", mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch("example.*/path?query=*", mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch("*.org*", mockUrl, mockOptions)).toBe(false);
      });
      it("handles empty string patterns", function () {
        expect(api.isMatch("", mockUrl, mockOptions)).toBe(false);
      });
    });
    describe("regex patterns", function () {
      it("matches regex patterns", function () {
        expect(api.isMatch(/example\.com/, mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch(/example\.com\/path\?query=value/, mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch(/different\.com/, mockUrl, mockOptions)).toBe(false);
      });
    });
    describe("function patterns", function () {
      it("matches function patterns", function () {
        expect(api.isMatch(function (url) { return url.hostname === "example.com"; }, mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch(function (url) { return url.hostname === "different.com"; }, mockUrl, mockOptions)).toBe(false);
      });
      it("passes options to function patterns", function () {
        var matchFn = function (url, options) { return options.opener && options.opener.name === "TestApp"; };
        expect(api.isMatch(matchFn, mockUrl, mockOptions)).toBe(true);
      });
    });
    describe("array patterns", function () {
      it("matches if any pattern in array matches", function () {
        expect(api.isMatch(["different.com", "example.com*"], mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch([/different\.com/, /example\.com/], mockUrl, mockOptions)).toBe(true);
        expect(api.isMatch(["different.com", /different\.org/], mockUrl, mockOptions)).toBe(false);
      });
      it("handles mixed pattern types in arrays", function () {
        var matchFn = function (url) { return url.hostname === "example.com"; };
        expect(api.isMatch(["different.com", /example\.com/, matchFn], mockUrl, mockOptions)).toBe(true);
      });
    });
  });

  describe("rewriteUrl", function () {
    var mockUrl = finickyURL("https://example.com/path?query=value#fragment");
    it("handles string rewrites", function () {
      var result = api.rewriteUrl("https://rewritten.com", mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://rewritten.com/");
    });
    it("handles URL object rewrites", function () {
      var result = api.rewriteUrl(new URL("https://rewritten.com/newpath"), mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://rewritten.com/newpath");
    });
    it("handles FinickyURL object rewrites", function () {
      var newUrl = finickyURL("https://rewritten.com/newpath", mockProcessInfo);
      var result = api.rewriteUrl(newUrl, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://rewritten.com/newpath");
    });
    it("handles function rewrites that return strings", function () {
      var result = api.rewriteUrl(function () { return "https://rewritten.com/function"; }, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://rewritten.com/function");
    });
    it("handles function rewrites that return URL objects", function () {
      var result = api.rewriteUrl(function () { return new URL("https://rewritten.com/function-url"); }, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://rewritten.com/function-url");
    });
    it("handles function rewrites that return FinickyURL objects", function () {
      var result = api.rewriteUrl(function () { return finickyURL("https://rewritten.com/function-finicky"); }, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://rewritten.com/function-finicky");
    });
    // vitest mocks isLegacyURLObject and legacyURLObjectToString; neither is
    // exported. Unmocked, an object without `username` is still a legacy URL
    // object, and the API writes the missing username as "undefined".
    it("handles legacy URL object rewrites", function () {
      var legacyUrl = { host: "legacy.com", protocol: "https", pathname: "/legacy-path", search: "query=value", hash: "fragment" };
      var result = api.rewriteUrl(legacyUrl, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://undefinedlegacy.com/legacy-path?query=value#fragment");
      var withUser = { username: "", host: "legacy.com", protocol: "https", pathname: "/legacy-path", search: "query=value", hash: "fragment" };
      expect(api.rewriteUrl(withUser, mockUrl, mockOptions).href).toBe("https://legacy.com/legacy-path?query=value#fragment");
    });
    it("returns the original URL for unsupported rewrite types", function () {
      expect(api.rewriteUrl(123, mockUrl, mockOptions)).toBe(mockUrl);
    });
    it("simulates chained function rewrites", function () {
      var result = api.rewriteUrl(function () { return "https://chained-rewritten.com"; }, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://chained-rewritten.com/");
    });
    it("handles recursive URL transformations", function () {
      var firstResult = api.rewriteUrl(function () { return new URL("https://first-transform.com"); }, mockUrl, mockOptions);
      expect(firstResult.href).toBe("https://first-transform.com/");
      var secondResult = api.rewriteUrl(function () { return "https://second-transform.com"; }, firstResult, mockOptions);
      expect(secondResult.href).toBe("https://second-transform.com/");
    });
    // vitest mocks isLegacyURLObject to false; a string result never reaches it.
    it("handles function rewrites that return legacy URL objects via mocking", function () {
      var result = api.rewriteUrl(function () { return "https://function-legacy.com/from-function"; }, mockUrl, mockOptions);
      expect(result).toBeFinickyURL();
      expect(result.href).toBe("https://function-legacy.com/from-function");
    });
  });

  describe("autodetectAppStringType", function () {
    it("detects app names", function () {
      expect(api.autodetectAppStringType("Google Chrome")).toBe("appName");
      expect(api.autodetectAppStringType("Firefox")).toBe("appName");
      expect(api.autodetectAppStringType("Safari")).toBe("appName");
      expect(api.autodetectAppStringType("Chrome 123")).toBe("appName");
    });
    it("detects bundle IDs", function () {
      expect(api.autodetectAppStringType("com.google.Chrome")).toBe("bundleId");
      expect(api.autodetectAppStringType("org.mozilla.firefox")).toBe("bundleId");
      expect(api.autodetectAppStringType("com.apple.Safari")).toBe("bundleId");
    });
    it("detects app paths", function () {
      expect(api.autodetectAppStringType("/Applications/Google Chrome.app")).toBe("path");
      expect(api.autodetectAppStringType("~/Applications/Firefox.app")).toBe("path");
      expect(api.autodetectAppStringType("/Users/username/Applications/Custom Browser.app")).toBe("path");
    });
    it("handles null input", function () {
      expect(api.autodetectAppStringType(null)).toBe("none");
    });
    it("defaults to appName for ambiguous cases", function () {
      expect(api.autodetectAppStringType("Invalid/Path")).toBe("appName");
      expect(api.autodetectAppStringType("com.example!invalid")).toBe("appName");
    });
  });

  describe("resolveBrowser", function () {
    var mockUrl = finickyURL("https://example.com");
    it("resolves string browser specifications", function () {
      expect(api.resolveBrowser("Firefox", mockUrl, mockOptions)).toMatchObject({ name: "Firefox", appType: "appName", url: "https://example.com/" });
    });
    it("resolves browser specifications with profiles", function () {
      expect(api.resolveBrowser("Chrome:Work", mockUrl, mockOptions)).toMatchObject({ name: "Chrome", profile: "Work", url: "https://example.com/" });
    });
    it("resolves object browser specifications", function () {
      var browserConfig = { name: "Google Chrome", profile: "Personal", openInBackground: true };
      expect(api.resolveBrowser(browserConfig, mockUrl, mockOptions)).toMatchObject({
        name: "Google Chrome",
        profile: "Personal",
        openInBackground: true,
        url: "https://example.com/",
      });
    });
    it("resolves function browser specifications that return strings", function () {
      expect(api.resolveBrowser(function () { return "Firefox"; }, mockUrl, mockOptions)).toMatchObject({ name: "Firefox", url: "https://example.com/" });
    });
    it("resolves function browser specifications that return objects", function () {
      var browserFn = function () { return { name: "Google Chrome", profile: "Work" }; };
      expect(api.resolveBrowser(browserFn, mockUrl, mockOptions)).toMatchObject({ name: "Google Chrome", profile: "Work", url: "https://example.com/" });
    });
    it("resolves null browser specifications to empty browser with 'none' type", function () {
      expect(api.resolveBrowser(null, mockUrl, mockOptions)).toMatchObject({ name: "", appType: "none", url: "https://example.com/" });
    });
    it("throws error for undefined browser specifications", function () {
      expect(function () { api.resolveBrowser(function () { return undefined; }, mockUrl, mockOptions); }).toThrow();
    });
    it("throws error for invalid browser specifications", function () {
      expect(function () { api.resolveBrowser(123, mockUrl, mockOptions); }).toThrow();
    });
    it("passes options to function browser specifications", function () {
      var browserFn = function (url, options) { return options.opener && options.opener.name === "TestApp" ? "Firefox" : "Chrome"; };
      expect(api.resolveBrowser(browserFn, mockUrl, mockOptions)).toMatchObject({ name: "Firefox", url: "https://example.com/" });
    });
  });

  // ---- PwrFinicky's config.test.ts additions (the bundle's source) -------
  file = "config.test.ts (PwrFinicky)";
  describe("candidate validation", function () {
    it("reports the failing configuration field", function () {
      expect(function () {
        api.assertValidConfig({ defaultBrowser: "Safari", handlers: [{ match: 123, browser: "Firefox" }] });
      }).toThrow("handlers");
    });
    it("validates callbacks without running them for synthetic inputs", function () {
      var calls = 0;
      var callback = function () { calls++; throw new Error("requires a real URL/opener"); };
      var config = { defaultBrowser: callback };
      expect(function () { api.assertValidConfig(config); }).notToThrow();
      expect(api.getConfigState(config).defaultBrowser).toBe("Function");
      expect(calls).toBe(0);
    });
  });

  // Runs every case; returns one result per case, in order.
  function run() {
    return cases.map(function (c) {
      try {
        c.body();
        return { file: c.file, name: c.name, ok: true };
      } catch (e) {
        return { file: c.file, name: c.name, ok: false, error: String(e && e.message !== undefined ? e.message : e) };
      }
    });
  }

  return { run: run };
})();
