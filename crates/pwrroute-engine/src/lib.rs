//! PwrRoute's rules engine for Finicky v4 configs.
//!
//! The engine evaluates Finicky's own configuration API
//! (`assets/finickyConfigAPI.js`, unchanged) in QuickJS, loads a user's
//! `.js` or `.ts` config into it, and asks the API which browser a URL goes
//! to. Matching, rewriting and validation are therefore Finicky's code, and
//! the compatibility tests in `tests/` hold the results to the Go host's.
//!
//! An [`Engine`] is single-threaded (QuickJS runtimes are not `Send`): create
//! one per config on the thread that resolves links, and replace it on reload.

mod host;
mod loader;
mod transform;

use std::{
    cell::Cell,
    fs,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use rquickjs::{
    CatchResultExt, CaughtError, Context, Ctx, Function, Module, Object, Runtime, Value,
    function::Rest,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub use host::{DefaultHost, Host, LogLevel, ModifierKeys, PowerInfo, SystemInfo};

/// Finicky's configuration API, as the Go host embeds it. See
/// `assets/README.md` for where it comes from.
const CONFIG_API: &str = include_str!("../assets/finickyConfigAPI.js");

/// PwrRoute's traced `openUrl`: Finicky's own, on a view of the config that
/// records which rules fired. See [`Engine::resolve_traced`].
const TRACED_OPEN_URL: &str = include_str!("traced_open_url.js");

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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

/// A [`Resolution`] with the rules that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracedResolution {
    /// Exactly what [`Engine::resolve`] returns for the same URL.
    pub resolution: Resolution,
    /// Every `rewrite` entry that matched, in the order it ran.
    pub rewrites: Vec<RewriteStep>,
    /// What chose the browser.
    pub chosen_by: ChosenBy,
}

/// One `rewrite` entry that matched.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RewriteStep {
    /// Its index in the config's `rewrite` array.
    pub rule: usize,
    /// The URL after it ran. It can equal the URL before.
    pub url: String,
}

/// What chose the browser in a [`TracedResolution`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChosenBy {
    /// `handlers[index]` matched first.
    Handler { index: usize },
    /// `defaultBrowser`: no handler matched, or a rule threw (then
    /// [`Resolution::error`] says why).
    Default,
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

/// A loaded config, ready to resolve URLs.
pub struct Engine {
    // Field order is drop order: the context must go before its runtime.
    context: Context,
    _runtime: Runtime,
    deadline: Rc<Cell<Option<Instant>>>,
    limits: Limits,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Loads and validates the config at `path`. A `.ts`, `.mts`, `.js` or
    /// `.mjs` config may be an ES module (`export default { … }`) or legacy
    /// CommonJS (`module.exports = { … }`).
    pub fn load(path: &Path, host: Rc<dyn Host>, limits: Limits) -> Result<Self> {
        let path = fs::canonicalize(path).with_context(|| path.display().to_string())?;
        let source = fs::read_to_string(&path).with_context(|| path.display().to_string())?;

        let runtime = Runtime::new()?;
        runtime.set_memory_limit(limits.memory_bytes);
        runtime.set_loader(loader::FsResolver, loader::OxcLoader);
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let watch = Rc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            watch.get().is_some_and(|d| Instant::now() >= d)
        })));
        let context = Context::full(&runtime)?;

        let engine = Self {
            context,
            _runtime: runtime,
            deadline,
            limits,
        };
        engine.with_deadline(limits.load, |ctx| {
            install_globals(&ctx, &host)?;
            load_config(&ctx, &path, &source)
        })?;
        Ok(engine)
    }

    /// Resolves one URL. `opener` is the app the link came from, when known.
    pub fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Resolution> {
        self.open_url(
            "JSON.stringify(finickyConfigAPI.openUrl(url, opener, originalUrl, finalConfig))",
            url,
            opener,
        )
    }

    /// Resolves one URL as [`resolve`](Self::resolve) does, and reports
    /// which `rewrite` entries matched and what chose the browser.
    ///
    /// This is still Finicky's `openUrl`, not a copy of it: it runs on a
    /// view of the config that passes every read through, except that a
    /// rule's `url` or `browser` (read only when that rule fired) and
    /// `defaultBrowser` come back wrapped in a function that notes the rule
    /// and then does what `openUrl` would have done with the real value.
    /// Matchers are not touched, and every config function runs once, in the
    /// same order, with the same arguments. `src/traced_open_url.js` gives
    /// the argument in full; `tests/finicky_compat.rs` holds both paths to
    /// the same results. An `Err` carries the same message; only the JS stack
    /// quoted under it differs.
    pub fn resolve_traced(&self, url: &str, opener: Option<&Opener>) -> Result<TracedResolution> {
        self.open_url(
            "JSON.stringify(pwrrouteOpenUrlTraced(url, opener, originalUrl, finalConfig))",
            url,
            opener,
        )
    }

    /// Sets the globals `openUrl` reads, evaluates `script` under the
    /// resolve deadline and parses the JSON it returns.
    fn open_url<T: DeserializeOwned>(
        &self,
        script: &'static str,
        url: &str,
        opener: Option<&Opener>,
    ) -> Result<T> {
        self.with_deadline(self.limits.resolve, |ctx| {
            let globals = ctx.globals();
            globals.set("url", url)?;
            globals.set("originalUrl", url)?;
            let opener = match opener {
                Some(o) => ctx.json_parse(serde_json::to_string(o)?)?,
                None => Value::new_null(ctx.clone()),
            };
            globals.set("opener", opener)?;
            let json: String = ctx
                .eval(script)
                .catch(&ctx)
                .map_err(|e| js_error("resolving the URL", &e))?;
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

/// `self`, `console` and the `finicky` global, then the config API itself.
fn install_globals(ctx: &Ctx<'_>, host: &Rc<dyn Host>) -> Result<()> {
    let globals = ctx.globals();
    globals.set("self", globals.clone())?;
    install_console(ctx, host)?;

    ctx.eval::<(), _>(CONFIG_API)
        .catch(ctx)
        .map_err(|e| js_error("evaluating the config API", &e))?;
    ctx.eval::<(), _>(TRACED_OPEN_URL)
        .catch(ctx)
        .map_err(|e| js_error("evaluating the traced resolve", &e))?;

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

/// Evaluates the config and leaves the validated `finalConfig` on the global
/// object, the same end state as the Go host's VM setup.
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

    globals.set("namespace", NAMESPACE)?;
    ctx.eval::<(), _>(
        "var finalConfig = finickyConfigAPI.getConfiguration(namespace);\
         finickyConfigAPI.assertValidConfig(finalConfig);",
    )
    .catch(ctx)
    .map_err(|e| js_error("validating the config", &e))?;
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
