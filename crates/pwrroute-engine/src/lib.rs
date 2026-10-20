//! PwrRoute's rules engine for Finicky v4 configs.
//!
//! The engine evaluates Finicky's own configuration API
//! (`assets/finickyConfigAPI.js`, unchanged) in QuickJS, loads a user's
//! `.js` or `.ts` config and the visual rules in `rules.json` into it, and
//! asks the API which browser a URL goes to. Matching, rewriting and
//! validation are therefore Finicky's code, and the compatibility tests in
//! `tests/` hold the results to the Go host's.
//!
//! - [`Engine`] loads [`Sources`] and resolves a [`Link`], or returns a
//!   [`Trace`] of how it did, or a [`Description`] of the config's rules.
//! - [`RulesFile`] is `rules.json`, the visual rules editor's model.
//! - [`ShortLinks`] expands short links over a [`Network`] before resolving.
//! - [`discovery`] finds the config and `rules.json` the way Finicky does.
//! - [`Reloader`] watches them and swaps in a new [`Engine`] only once it has
//!   loaded and validated.
//! - [`NativeHost`] answers the `finicky.*` native functions from the OS.
//!
//! An [`Engine`] is single-threaded (QuickJS runtimes are not `Send`): create
//! one per config on the thread that resolves links, and replace it on reload.

mod describe;
pub mod discovery;
mod host;
#[cfg(feature = "http")]
mod http;
mod loader;
mod native;
mod reload;
mod rules;
mod shortlinks;
mod trace;
mod transform;

use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use rquickjs::{
    CatchResultExt, CaughtError, Context, Ctx, Function, Module, Object, Runtime, Value,
    function::Rest,
};
use serde::{Deserialize, Serialize};

pub use describe::{
    BrowserSpec, Description, FixedBrowser, FunctionSpec, HandlerRule, MatcherSpec, RewriteRule,
    UrlSpec,
};
pub use host::{DefaultHost, Host, LogLevel, ModifierKeys, PowerInfo, SystemInfo};
#[cfg(feature = "http")]
pub use http::HttpNetwork;
pub use native::NativeHost;
pub use reload::{ConfigChoice, Reloader, Status, WatchSet, Watcher};
pub use rules::{
    FALLBACK_BROWSER, Place, Problem, Rewrite, RewriteAction, Rule, RulesFile, RulesOptions,
};
pub use shortlinks::{DEFAULT_SHORTENERS, Link, Method, Network, Reply, ShortLinks};
pub use trace::{Matcher, Source, Stage, Step, Trace};

/// Finicky's configuration API, as the Go host embeds it. See
/// `assets/README.md` for where it comes from.
const CONFIG_API: &str = include_str!("../assets/finickyConfigAPI.js");

/// PwrRoute's glue: `rules.json` as Finicky objects, the merged config, and
/// the trace. Uses only the API's exports.
const HOST_GLUE: &str = include_str!("js/host.js");

/// The global a config's exports land on. `getConfiguration` reads
/// `self[namespace]`, preferring its `default` export.
const NAMESPACE: &str = "finickyConfig";

/// Limits that keep a broken config from hanging or exhausting the router.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Evaluating the config and validating it.
    pub load: Duration,
    /// Resolving one URL, including every matcher and rewrite function.
    pub resolve: Duration,
    /// The QuickJS heap.
    pub memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            load: Duration::from_secs(5),
            resolve: Duration::from_secs(1),
            memory_bytes: 128 * 1024 * 1024,
        }
    }
}

/// The app that handed the link to the OS, as matchers see it in
/// `options.opener`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Opener {
    pub name: String,
    pub bundle_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
}

/// Where a URL goes, as Finicky's `openUrl` decides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub browser: Browser,
    /// Set when a matcher, rewrite or browser function threw. Finicky still
    /// returns a browser in that case (the default), and so does the engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The browser and how to open it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    /// An app name, bundle id or path, as `app_type` says.
    pub name: String,
    pub app_type: AppType,
    /// The URL after every rewrite.
    pub url: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub open_in_background: Option<bool>,
}

/// How [`Browser::name`] identifies the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppType {
    AppName,
    BundleId,
    Path,
    /// `browser: null` in the config: do not open the URL at all.
    None,
}

/// What an [`Engine`] is built from: a JS or TS config, `rules.json`, or
/// both. With both, the config's rewrites and handlers run first and it owns
/// the default browser and options; the rules' rewrites and handlers follow
/// its own, as the Go host appended them. With neither, the default browser
/// is [`FALLBACK_BROWSER`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    pub config: Option<PathBuf>,
    pub rules: RulesFile,
}

/// A loaded config, ready to resolve URLs.
pub struct Engine {
    // Field order is drop order: the context must go before its runtime.
    context: Context,
    _runtime: Runtime,
    deadline: Rc<Cell<Option<Instant>>>,
    limits: Limits,
    config: Option<PathBuf>,
    imports: Rc<RefCell<Vec<PathBuf>>>,
    short_links: ShortLinks,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("config", &self.config)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Loads and validates the config at `path`. A `.ts`, `.mts`, `.js` or
    /// `.mjs` config may be an ES module (`export default { … }`) or legacy
    /// CommonJS (`module.exports = { … }`).
    pub fn load(path: &Path, host: Rc<dyn Host>, limits: Limits) -> Result<Self> {
        Self::from_sources(
            &Sources {
                config: Some(path.to_path_buf()),
                rules: RulesFile::default(),
            },
            host,
            limits,
        )
    }

    /// Loads a config, `rules.json`, or both, and validates the result.
    pub fn from_sources(sources: &Sources, host: Rc<dyn Host>, limits: Limits) -> Result<Self> {
        Self::from_sources_reading(sources, host, limits, &Rc::default())
    }

    /// [`Engine::from_sources`], recording each file the config imports in
    /// `imports` as it is read, so a caller still knows them when the load
    /// fails.
    pub(crate) fn from_sources_reading(
        sources: &Sources,
        host: Rc<dyn Host>,
        limits: Limits,
        imports: &Rc<RefCell<Vec<PathBuf>>>,
    ) -> Result<Self> {
        let config = match &sources.config {
            Some(path) => {
                let path = fs::canonicalize(path).with_context(|| path.display().to_string())?;
                let source =
                    fs::read_to_string(&path).with_context(|| path.display().to_string())?;
                Some((path, source))
            }
            None => None,
        };

        let runtime = Runtime::new()?;
        runtime.set_memory_limit(limits.memory_bytes);
        let imports = Rc::clone(imports);
        runtime.set_loader(
            loader::FsResolver,
            loader::OxcLoader {
                loaded: Rc::clone(&imports),
            },
        );
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let watch = Rc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            watch.get().is_some_and(|d| Instant::now() >= d)
        })));
        let context = Context::full(&runtime)?;

        let mut engine = Self {
            context,
            _runtime: runtime,
            deadline,
            limits,
            config: config.as_ref().map(|(path, _)| path.clone()),
            imports,
            short_links: ShortLinks::default(),
        };
        let shorteners = engine.with_deadline(limits.load, |ctx| {
            install_globals(&ctx, &host)?;
            let base = match &config {
                Some(_) => serde_json::Value::Null,
                None => sources.rules.base_config(),
            };
            let setup: Function<'_> = ctx.eval("__pwrroute.setup")?;
            setup
                .call::<_, ()>((
                    ctx.json_parse(sources.rules.plan().to_string())?,
                    ctx.json_parse(base.to_string())?,
                ))
                .catch(&ctx)
                .map_err(|e| js_error("loading rules.json", &e))?;
            if let Some((path, source)) = &config {
                load_config(&ctx, path, source)?;
            }
            adopt_config(&ctx, config.is_some())?;
            let list: Vec<String> = ctx
                .eval::<Vec<String>, _>("__pwrroute.shorteners()")
                .catch(&ctx)
                .map_err(|e| js_error("reading options.urlShorteners", &e))?;
            Ok(list)
        })?;
        engine.short_links = ShortLinks::with_domains(shorteners);
        Ok(engine)
    }

    /// The config file in charge, canonicalized, when there is one.
    pub fn config_path(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// The config file and every file it imported, so a watcher can reload
    /// when any of them changes.
    pub fn dependencies(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.config.iter().cloned().collect();
        for path in self.imports.borrow().iter() {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        paths
    }

    /// Which links this config expands before resolving: the built-in
    /// shorteners plus the config's `options.urlShorteners`. Cloneable and
    /// `Send`, so expansion can run off the engine's thread.
    pub fn short_links(&self) -> &ShortLinks {
        &self.short_links
    }

    /// Resolves one URL as it is, without short-link expansion. `opener` is
    /// the app the link came from, when known.
    pub fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Resolution> {
        self.resolve_link(&Link::new(url), opener)
    }

    /// Expands `url` if it is a short link, then resolves it, as the Go host
    /// did for every link.
    pub fn expand_and_resolve(
        &self,
        url: &str,
        opener: Option<&Opener>,
        network: &dyn Network,
    ) -> Result<Resolution> {
        self.resolve_link(&self.short_links.expand(url, network), opener)
    }

    /// Resolves a link whose short link (if any) was already expanded.
    pub fn resolve_link(&self, link: &Link, opener: Option<&Opener>) -> Result<Resolution> {
        self.with_deadline(self.limits.resolve, |ctx| {
            set_link(&ctx, link, opener)?;
            let json: String = ctx
                .eval(
                    "JSON.stringify(finickyConfigAPI.openUrl(url, opener, originalUrl, __pwrroute.config()))",
                )
                .catch(&ctx)
                .map_err(|e| js_error("resolving the URL", &e))?;
            Ok(serde_json::from_str(&json)?)
        })
    }

    /// Resolves a link and returns every step on the way: the rewrites and
    /// handlers evaluated, which matched, and which won. Nothing is opened;
    /// the resolution is the one [`Engine::resolve_link`] returns.
    pub fn trace(&self, link: &Link, opener: Option<&Opener>) -> Result<Trace> {
        self.with_deadline(self.limits.resolve, |ctx| {
            set_link(&ctx, link, opener)?;
            let json: String = ctx
                .eval("JSON.stringify(__pwrroute.trace(url, opener, originalUrl))")
                .catch(&ctx)
                .map_err(|e| js_error("resolving the URL", &e))?;
            let raw: trace::Raw = serde_json::from_str(&json)?;
            Ok(Trace {
                url: link.url.clone(),
                original_url: link.original_url.clone(),
                steps: raw.steps,
                resolution: raw.result,
            })
        })
    }

    /// The JS or TS config's own rewrites, handlers and default browser, in
    /// the order `openUrl` runs them. `None` when `rules.json` is the whole
    /// config; [`RulesFile`] describes its rules. Runs under the resolve
    /// deadline, since reading a config's objects can run its getters.
    pub fn describe(&self) -> Result<Option<Description>> {
        if self.config.is_none() {
            return Ok(None);
        }
        self.with_deadline(self.limits.resolve, |ctx| {
            let json: String = ctx
                .eval("JSON.stringify(__pwrroute.describe())")
                .catch(&ctx)
                .map_err(|e| js_error("describing the config", &e))?;
            Ok(serde_json::from_str(&json)?)
        })
    }

    fn with_deadline<T>(
        &self,
        budget: Duration,
        f: impl FnOnce(Ctx<'_>) -> Result<T>,
    ) -> Result<T> {
        self.deadline.set(Some(Instant::now() + budget));
        let result = self.context.with(f);
        self.deadline.set(None);
        result
    }
}

/// The `url`, `originalUrl` and `opener` globals `openUrl` reads.
fn set_link(ctx: &Ctx<'_>, link: &Link, opener: Option<&Opener>) -> Result<()> {
    let globals = ctx.globals();
    globals.set("url", link.url.as_str())?;
    globals.set("originalUrl", link.original_url.as_str())?;
    let opener = match opener {
        Some(o) => ctx.json_parse(serde_json::to_string(o)?)?,
        None => Value::new_null(ctx.clone()),
    };
    globals.set("opener", opener)?;
    Ok(())
}

/// `self`, `console` and the `finicky` global, then the config API itself.
fn install_globals(ctx: &Ctx<'_>, host: &Rc<dyn Host>) -> Result<()> {
    let globals = ctx.globals();
    globals.set("self", globals.clone())?;
    install_console(ctx, host)?;

    ctx.eval::<(), _>(CONFIG_API)
        .catch(ctx)
        .map_err(|e| js_error("evaluating the config API", &e))?;
    ctx.eval::<(), _>(HOST_GLUE)
        .catch(ctx)
        .map_err(|e| js_error("evaluating the host glue", &e))?;

    // `finicky` is the API's utilities plus the host's native functions, as
    // the Go host assembled it.
    let api: Object<'_> = globals.get("finickyConfigAPI")?;
    let utilities: Object<'_> = api.get("utilities")?;
    let finicky = Object::new(ctx.clone())?;
    for key in utilities.keys::<String>() {
        let key = key?;
        let value: Value<'_> = utilities.get(&key)?;
        finicky.set(key, value)?;
    }
    install_host_functions(ctx, &finicky, host)?;
    globals.set("finicky", finicky)?;
    Ok(())
}

fn install_host_functions<'js>(
    ctx: &Ctx<'js>,
    finicky: &Object<'js>,
    host: &Rc<dyn Host>,
) -> Result<()> {
    fn as_js<'js>(ctx: &Ctx<'js>, value: &impl Serialize) -> rquickjs::Result<Value<'js>> {
        let json = serde_json::to_string(value)
            .map_err(|e| rquickjs::Error::new_from_js_message("value", "object", e.to_string()))?;
        ctx.json_parse(json)
    }

    let h = Rc::clone(host);
    finicky.set(
        "getModifierKeys",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            as_js(&ctx, &h.modifier_keys())
        })?,
    )?;
    let h = Rc::clone(host);
    finicky.set(
        "getSystemInfo",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            as_js(&ctx, &h.system_info())
        })?,
    )?;
    let h = Rc::clone(host);
    finicky.set(
        "getPowerInfo",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            as_js(&ctx, &h.power_info())
        })?,
    )?;
    let h = Rc::clone(host);
    finicky.set(
        "isAppRunning",
        Function::new(ctx.clone(), move |identifier: String| {
            h.is_app_running(&identifier)
        })?,
    )?;
    Ok(())
}

fn install_console<'js>(ctx: &Ctx<'js>, host: &Rc<dyn Host>) -> Result<()> {
    let console = Object::new(ctx.clone())?;
    for (name, level) in [
        ("debug", LogLevel::Debug),
        ("log", LogLevel::Info),
        ("info", LogLevel::Info),
        ("warn", LogLevel::Warn),
        ("error", LogLevel::Error),
    ] {
        let h = Rc::clone(host);
        console.set(
            name,
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                let message = args
                    .0
                    .iter()
                    .map(|v| display(&ctx, v))
                    .collect::<Vec<_>>()
                    .join(" ");
                h.log(level, &message);
            })?,
        )?;
    }
    ctx.globals().set("console", console)?;
    Ok(())
}

/// How `console.log` shows a value: strings as-is, everything else as JSON
/// where possible.
fn display<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
    if let Some(s) = value.as_string() {
        return s.to_string().unwrap_or_default();
    }
    ctx.json_stringify(value.clone())
        .ok()
        .flatten()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| format!("{value:?}"))
}

/// Evaluates the config and leaves its exports on `self[NAMESPACE]`.
fn load_config(ctx: &Ctx<'_>, path: &Path, source: &str) -> Result<()> {
    let globals = ctx.globals();
    let transformed = transform::transform(path, source)?;
    if transformed.is_module {
        // Declared from the transformed text; its own imports go through
        // OxcLoader. The namespace object carries `default`, which
        // getConfiguration looks for first.
        let name = path.to_string_lossy().into_owned();
        let (module, promise) = Module::declare(ctx.clone(), name, transformed.code)
            .and_then(Module::eval)
            .catch(ctx)
            .map_err(|e| js_error("loading the config", &e))?;
        promise
            .finish::<()>()
            .catch(ctx)
            .map_err(|e| js_error("loading the config", &e))?;
        let namespace = module
            .namespace()
            .catch(ctx)
            .map_err(|e| js_error("loading the config", &e))?;
        globals.set(NAMESPACE, namespace)?;
    } else {
        // Legacy `module.exports = …`. getConfiguration accepts the object
        // itself and warns, as it does under Go.
        let wrapped = format!(
            "(function(){{var module={{exports:{{}}}};var exports=module.exports;\n{}\n;return module.exports;}})()",
            transformed.code
        );
        let exports: Value<'_> = ctx
            .eval(wrapped)
            .catch(ctx)
            .map_err(|e| js_error("loading the config", &e))?;
        globals.set(NAMESPACE, exports)?;
    }

    Ok(())
}

/// Leaves the validated `finalConfig` on the global object, the same end
/// state as the Go host's VM setup, and hands it to the glue. The config is
/// validated on its own first, so its errors read as they did under Go; then
/// with `rules.json` appended, so a bad rule fails the load rather than every
/// resolve.
fn adopt_config(ctx: &Ctx<'_>, has_config: bool) -> Result<()> {
    ctx.globals().set("namespace", NAMESPACE)?;
    ctx.eval::<(), _>(
        "var finalConfig = finickyConfigAPI.getConfiguration(namespace);\
         finickyConfigAPI.assertValidConfig(finalConfig);\
         __pwrroute.adopt(finalConfig);",
    )
    .catch(ctx)
    .map_err(|e| {
        js_error(
            if has_config {
                "validating the config"
            } else {
                "validating rules.json"
            },
            &e,
        )
    })?;
    ctx.eval::<(), _>("finickyConfigAPI.assertValidConfig(__pwrroute.config());")
        .catch(ctx)
        .map_err(|e| js_error("validating rules.json", &e))?;
    Ok(())
}

/// A JS exception as an error message. QuickJS reports an interrupt as an
/// uncatchable "interrupted" error, which is the deadline firing.
fn js_error(stage: &str, error: &CaughtError<'_>) -> anyhow::Error {
    let text = error.to_string();
    if text.contains("interrupted") {
        anyhow!("{stage}: timed out")
    } else {
        anyhow!("{stage}: {text}")
    }
}
