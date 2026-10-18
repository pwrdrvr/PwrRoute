// PwrRoute's traced resolve: Finicky's own `openUrl`, run on a view of the
// config that records which rules fired. Evaluated once, after the config API.
//
// The view is a Proxy that passes every read through to the config, except
// three properties `openUrl` reads only when a rule fires:
//
// - `rewrite[i].url`, read after `rewrite[i].match` matched. The view returns
//   a function that calls the API's own `rewriteUrl` on the real value, notes
//   the URL it produced, and returns it. `openUrl` hands that function to
//   `rewriteUrl`, which calls it and returns its URL unchanged.
// - `handlers[i].browser`, read after `handlers[i].match` matched. The view
//   returns a function that notes the handler and returns what
//   `resolveBrowser` would have taken from the real value: the result of
//   calling it if it is a function, the value itself otherwise.
// - `defaultBrowser`, read when no handler matched or a rule threw. Wrapped
//   the same way.
//
// Matchers are untouched, and each user function runs once, with the same
// arguments, in the same order as under `openUrl` on the config itself.
var pwrrouteOpenUrlTraced = (function (api) {
  "use strict";

  const { assertValidConfig, openUrl, rewriteUrl } = api;
  const isIndex = (key) => typeof key === "string" && /^(0|[1-9][0-9]*)$/.test(key);

  function browserFrom(spec, record) {
    return function (url, options) {
      record();
      return typeof spec === "function" ? spec(url, options) : spec;
    };
  }

  // What a Proxy over `target` can stand on. A Proxy may not report a
  // different value for a property that cannot change (non-configurable and
  // non-writable, as on a frozen object), so when one of `keys` is such a
  // property the Proxy gets a configurable copy as its target. Reads still go
  // to `target` itself; the copy only answers the Proxy's invariant checks.
  function proxyTarget(target, keys) {
    const pinned = keys.some((key) => {
      const d = Reflect.getOwnPropertyDescriptor(target, key);
      return d !== undefined && !d.configurable && ("value" in d ? !d.writable : d.get === undefined);
    });
    if (!pinned) return target;
    const isArray = Array.isArray(target);
    const copy = Object.setPrototypeOf(isArray ? [] : {}, Object.getPrototypeOf(target));
    for (const key of Reflect.ownKeys(target)) {
      if (isArray && key === "length") continue;
      const d = Reflect.getOwnPropertyDescriptor(target, key);
      Object.defineProperty(copy, key, { ...d, configurable: true });
    }
    if (isArray) copy.length = target.length;
    return copy;
  }

  // A view of `target` whose properties named in `replacements` read through
  // `replacements[key](value)`.
  function replacing(target, replacements) {
    return new Proxy(proxyTarget(target, Object.keys(replacements)), {
      get(_, key) {
        const value = Reflect.get(target, key);
        return Object.prototype.hasOwnProperty.call(replacements, key)
          ? replacements[key](value)
          : value;
      },
    });
  }

  // A view of a rules array whose entries read through `rule(entry, index)`.
  function rules(list, rule) {
    if (!Array.isArray(list)) return list;
    const indexes = Array.from({ length: list.length }, (_, i) => String(i));
    return new Proxy(proxyTarget(list, indexes), {
      get(_, key) {
        const value = Reflect.get(list, key);
        return isIndex(key) && value !== null && typeof value === "object"
          ? rule(value, Number(key))
          : value;
      },
    });
  }

  return function (urlString, opener, originalUrlString, config) {
    const trace = { rewrites: [], chosenBy: null };

    // The view's replacements are functions, which validate where some
    // invalid values would not. `openUrl` rejects an invalid config before
    // any rule runs, so give it such a config as it is.
    try {
      assertValidConfig(config);
    } catch (_) {
      return { resolution: openUrl(urlString, opener, originalUrlString, config), ...trace };
    }

    const view = replacing(config, {
      rewrite: (list) =>
        rules(list, (rule, index) =>
          replacing(rule, {
            url: (spec) =>
              function (url, options) {
                const out = rewriteUrl(spec, url, options);
                trace.rewrites.push({ rule: index, url: out.href });
                return out;
              },
          }),
        ),
      handlers: (list) =>
        rules(list, (rule, index) =>
          replacing(rule, {
            browser: (spec) =>
              browserFrom(spec, () => {
                trace.chosenBy = { kind: "handler", index };
              }),
          }),
        ),
      defaultBrowser: (spec) =>
        browserFrom(spec, () => {
          trace.chosenBy = { kind: "default" };
        }),
    });

    const resolution = openUrl(urlString, opener, originalUrlString, view);
    return { resolution, ...trace };
  };
})(finickyConfigAPI);
