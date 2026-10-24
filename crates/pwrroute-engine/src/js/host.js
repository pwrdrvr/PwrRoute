// PwrRoute's host glue, evaluated after finickyConfigAPI.js. It uses only the
// API's exports and never reimplements its decisions: openUrl still runs the
// rewrites and picks the handler, here and when tracing.
var __pwrroute = (function (api) {
  "use strict";

  var finalConfig = null; // what the config (or rules.json alone) produced
  var json = { handlers: [], rewrite: [] }; // rules.json, as Finicky objects
  var jsonOnly = false;
  // With a JS config in charge and "precedence": "pwrroute": rules.json's
  // handlers run before the config's, and, when rules.json names a usable
  // default (`ownDefault`), it takes everything else instead of the
  // config's defaultBrowser. Rewrites keep the config's first either way.
  var rulesFirst = false;
  var ownDefault = null; // { browser: spec } when rules.json owns the default
  var described = { handlers: [], rewrite: [] }; // matchers of the JSON rules

  function sameApp(names, opener) {
    if (!opener) return false;
    var name = String(opener.name || "").toLowerCase();
    var id = String(opener.bundleId || "").toLowerCase();
    for (var i = 0; i < names.length; i++) {
      var want = String(names[i]).toLowerCase();
      if (want && (want === name || want === id)) return true;
    }
    return false;
  }

  function globToRegExp(glob) {
    var parts = String(glob).split("*").map(function (p) {
      return p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    });
    return new RegExp("^" + parts.join(".*") + "$");
  }

  function handler(rule) {
    if (!rule.opener) return { match: rule.match, browser: rule.browser };
    var match = rule.match;
    var names = rule.opener;
    return {
      match: function (url, options) {
        return sameApp(names, options && options.opener) && api.isMatch(match, url, options);
      },
      browser: rule.browser,
    };
  }

  function apply(action, url, options) {
    if ("url" in action) return api.rewriteUrl(action.url, url, options);
    if ("setScheme" in action) url.protocol = action.setScheme + ":";
    else if ("setHost" in action) url.host = action.setHost;
    else if ("replacePath" in action) url.pathname = action.replacePath;
    else if ("dropQuery" in action) {
      var drop = action.dropQuery.map(globToRegExp);
      var keys = [];
      url.searchParams.forEach(function (_, key) {
        keys.push(key);
      });
      keys.forEach(function (key) {
        if (drop.some(function (re) { return re.test(key); })) url.searchParams.delete(key);
      });
    }
    return url;
  }

  function rewrite(rule) {
    var actions = rule.actions;
    return {
      match: rule.match,
      url: function (url, options) {
        for (var i = 0; i < actions.length; i++) url = apply(actions[i], url, options);
        return url;
      },
    };
  }

  function describe(match) {
    if (Array.isArray(match)) return match.map(describeOne);
    return [describeOne(match)];
  }

  function describeOne(match) {
    if (typeof match === "string") return { wildcard: match };
    if (match instanceof RegExp) return { regex: String(match) };
    if (typeof match === "function") return { function: match.name || "" };
    return { other: typeof match };
  }

  // A function's name and, when QuickJS kept it, its source text. Read with
  // Function.prototype.toString so a config cannot answer for its own
  // functions; a native or bound function has no source to show.
  function describeFunction(fn) {
    var text = null;
    try {
      text = Function.prototype.toString.call(fn);
    } catch (ex) {
      text = null;
    }
    if (text !== null && /\{\s*\[native code\]\s*\}\s*$/.test(text)) text = null;
    var described = { name: fn.name || "", source: text };
    var hostnames = describeHostnames(fn);
    if (hostnames) described.hostnames = hostnames;
    return described;
  }

  // finicky.matchHostnames (and the deprecated matchDomains, which calls
  // it) returns a function whose source is the API's, not the list it was
  // given. The wrappers remember each list by the function the API
  // returned, unchanged, so describe() can show the hostnames.
  var hostnameLists = typeof WeakMap === "function" ? new WeakMap() : null;

  function rememberHostnames(finicky) {
    if (!hostnameLists || !finicky) return;
    ["matchHostnames", "matchDomains"].forEach(function (key) {
      var original = finicky[key];
      if (typeof original !== "function") return;
      finicky[key] = function (matchers) {
        var fn = original.apply(this, arguments);
        if (typeof fn === "function") hostnameLists.set(fn, [].concat(matchers));
        return fn;
      };
    });
  }

  function describeHostnames(fn) {
    var list = hostnameLists && hostnameLists.get(fn);
    if (!list) return null;
    return list.map(function (entry) {
      if (entry instanceof RegExp) return { regex: String(entry) };
      return { hostname: String(entry) };
    });
  }

  function describeMatcher(match) {
    if (typeof match === "function") return { function: describeFunction(match) };
    return describeOne(match);
  }

  // A browser as the API's createBrowserConfig spells a fixed one out
  // ("Firefox:work" is Firefox with the work profile); a function is known
  // only by its source until it runs.
  function describeBrowser(spec) {
    if (typeof spec === "function") return { function: describeFunction(spec) };
    var b = api.createBrowserConfig(spec);
    return {
      browser: {
        name: String(b.name || ""),
        appType: b.appType,
        profile: String(b.profile || ""),
        args: (b.args || []).map(String),
        openInBackground: typeof b.openInBackground === "boolean" ? b.openInBackground : null,
      },
    };
  }

  function describeTarget(spec) {
    if (typeof spec === "function") return { function: describeFunction(spec) };
    // A string, or a URL object, which carries its href.
    return { url: spec && typeof spec.href === "string" ? spec.href : String(spec) };
  }

  // The config's own rewrites and handlers, in the order openUrl runs them,
  // and its default browser. rules.json's are not included: RulesFile is
  // their description.
  function description() {
    if (jsonOnly) return null;
    return {
      rewrites: (finalConfig.rewrite || []).map(function (r) {
        return { matchers: [].concat(r.match).map(describeMatcher), url: describeTarget(r.url) };
      }),
      handlers: (finalConfig.handlers || []).map(function (h) {
        return {
          matchers: [].concat(h.match).map(describeMatcher),
          browser: describeBrowser(h.browser),
        };
      }),
      defaultBrowser: describeBrowser(finalConfig.defaultBrowser),
    };
  }

  // The config openUrl sees. With a JS config, rules.json's handlers come
  // after the config's own, exactly as the Go host's resolver appended them,
  // or before them under rulesFirst; rules.json's default replaces the
  // config's (fixed, null or a function) when it owns it. Rewrites always
  // run the config's first.
  function config() {
    if (jsonOnly) return finalConfig;
    var own = finalConfig.handlers || [];
    var extra = {
      handlers: rulesFirst ? json.handlers.concat(own) : own.concat(json.handlers),
    };
    if (json.rewrite.length) extra.rewrite = (finalConfig.rewrite || []).concat(json.rewrite);
    if (ownDefault) extra.defaultBrowser = ownDefault.browser;
    return Object.assign({}, finalConfig, extra);
  }

  function message(ex) {
    return ex instanceof Error ? ex.message : String(ex);
  }

  // openUrl over a copy of the config whose matchers, rewrites and browsers
  // record what they did before deferring to the API's own isMatch,
  // rewriteUrl and browser handling. openUrl calls isMatch(fn) as fn(url,
  // options), rewriteUrl(fn) as rewriteUrl(fn(url, options)), and
  // resolveBrowser(fn) as resolveBrowser(fn(url, options)), so each wrapper
  // returns what the original would have produced through the same path.
  function trace(url, opener, originalUrl) {
    var cfg = config();
    var steps = [];
    var counts = jsonOnly
      ? { rewrite: 0, handlers: 0 }
      : {
          rewrite: (finalConfig.rewrite || []).length,
          handlers: (finalConfig.handlers || []).length,
        };

    // Which list an entry of the merged config came from, and its place
    // there. Under rulesFirst rules.json's handlers lead.
    function origin(kind, index) {
      if (kind === "handlers" && rulesFirst) {
        var lead = json.handlers.length;
        if (index < lead) return { source: "rules", index: index };
        return { source: "config", index: index - lead };
      }
      if (index < counts[kind]) return { source: "config", index: index };
      return { source: "rules", index: index - counts[kind] };
    }

    function matchers(kind, index, match) {
      var from = origin(kind, index);
      if (from.source === "rules") return described[kind][from.index] || describe(match);
      return describe(match);
    }

    function wrapMatch(stage, kind, index, match) {
      var from = origin(kind, index);
      return function (url, options) {
        var step = {
          stage: stage,
          source: from.source,
          index: from.index,
          matchers: matchers(kind, index, match),
          url: url.href,
          matched: false,
        };
        steps.push(step);
        var result;
        try {
          if (Array.isArray(match)) {
            result = false;
            for (var i = 0; i < match.length; i++) {
              if (api.isMatch(match[i], url, options)) {
                result = true;
                step.pattern = i;
                break;
              }
            }
          } else {
            result = api.isMatch(match, url, options);
          }
        } catch (ex) {
          step.error = message(ex);
          throw ex;
        }
        step.matched = !!result;
        return result;
      };
    }

    function wrapRewrite(target) {
      return function (url, options) {
        var step = steps[steps.length - 1];
        try {
          var next = api.rewriteUrl(target, url, options);
          step.rewrittenTo = next.href;
          return next;
        } catch (ex) {
          step.error = message(ex);
          throw ex;
        }
      };
    }

    function wrapBrowser(spec, onUse) {
      return function (url, options) {
        onUse(url);
        return typeof spec === "function" ? spec(url, options) : spec;
      };
    }

    var traced = Object.assign({}, cfg);
    if (cfg.rewrite) {
      traced.rewrite = cfg.rewrite.map(function (r, i) {
        return Object.assign({}, r, {
          match: wrapMatch("rewrite", "rewrite", i, r.match),
          url: wrapRewrite(r.url),
        });
      });
    }
    if (cfg.handlers) {
      traced.handlers = cfg.handlers.map(function (h, i) {
        return Object.assign({}, h, {
          match: wrapMatch("handler", "handlers", i, h.match),
          browser: wrapBrowser(h.browser, function () {
            steps[steps.length - 1].won = true;
          }),
        });
      });
    }
    traced.defaultBrowser = wrapBrowser(cfg.defaultBrowser, function (url) {
      steps.push({
        stage: "default",
        // rules.json's default when it is the whole config or owns it.
        source: jsonOnly || ownDefault ? "rules" : "config",
        index: 0,
        matchers: [],
        url: url.href,
        matched: true,
        won: true,
      });
    });

    var result = api.openUrl(url, opener, originalUrl, traced);
    // A handler's browser is marked when openUrl asks for it, before
    // resolveBrowser validates it. If the default was used after all, that
    // handler's browser failed: it did not win, and the error is its own.
    if (steps.some(function (s) { return s.stage === "default"; })) {
      steps.forEach(function (s) {
        if (s.stage !== "default" && s.won) {
          s.won = false;
          if (result.error) s.error = result.error;
        }
      });
    }
    return { steps: steps, result: result };
  }

  // Which of `urls` `pattern` would match, by the API's own isMatch, as a
  // handler with that pattern would. A URL that does not parse never matches.
  function matches(pattern, urls) {
    return urls.map(function (u) {
      var url;
      try {
        url = new URL(u);
      } catch (ex) {
        return false;
      }
      return !!api.isMatch(pattern, url, {});
    });
  }

  // For each of `urls`, whether each of the config's own handlers matches
  // it. One openUrl run per link calls every matcher through the API's
  // isMatch with what routing passes it (a FinickyURL and the options
  // openUrl builds, here without an opener); each wrapper records the
  // answer and declines, so no rewrite applies and no handler wins. A
  // matcher that throws, or a link openUrl rejects, counts as no match.
  function probe(urls) {
    var handlers = finalConfig.handlers || [];
    return urls.map(function (u) {
      var row = handlers.map(function () {
        return false;
      });
      var cfg = {
        defaultBrowser: "Safari",
        handlers: handlers.map(function (h, i) {
          return {
            match: function (url, options) {
              try {
                row[i] = !!api.isMatch(h.match, url, options);
              } catch (ex) {
                row[i] = false;
              }
              return false;
            },
            browser: "Safari",
          };
        }),
      };
      try {
        api.openUrl(u, null, null, cfg);
      } catch (ex) {
        // An invalid link: every row stays false.
      }
      return row;
    });
  }

  return {
    // Called once at load. `rules` is RulesFile::plan(); `base` is set when
    // rules.json is the whole config, and then its precedence is moot.
    setup: function (rules, base) {
      json = {
        handlers: rules.handlers.map(handler),
        rewrite: rules.rewrites.map(rewrite),
      };
      described = {
        handlers: rules.handlers.map(function (h) {
          return describe(h.match);
        }),
        rewrite: rules.rewrites.map(function (r) {
          return describe(r.match);
        }),
      };
      if (base) {
        jsonOnly = true;
        base.handlers = json.handlers;
        if (json.rewrite.length) base.rewrite = json.rewrite;
        self.finickyConfig = { default: base };
      } else {
        rulesFirst = !!rules.rulesFirst;
        if (rules.defaultBrowser !== undefined) ownDefault = { browser: rules.defaultBrowser };
      }
    },
    // Called once finalConfig exists.
    adopt: function (config) {
      finalConfig = config;
    },
    config: config,
    trace: trace,
    describe: description,
    rememberHostnames: rememberHostnames,
    matches: matches,
    probe: probe,
    // `options.urlShorteners` as the config set it: null when it did not,
    // so the host can tell "use the built-in list" from "expand nothing".
    shorteners: function () {
      var list = api.getOption("urlShorteners", finalConfig, null);
      if (list === null || list === undefined) return null;
      return Array.isArray(list) ? list.map(String) : [];
    },
  };
})(finickyConfigAPI);
