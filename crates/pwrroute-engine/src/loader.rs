//! Module resolution and loading for config imports.
//!
//! The Go host bundled each config with esbuild before evaluating it. Here
//! QuickJS asks for each import as it meets it, and every file goes through
//! oxc on the way in, so a config can import its own `.ts` helpers.

use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use rquickjs::{
    Ctx, Module,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};

use crate::transform::transform;

/// Extensions tried, in order, for an import written without one.
const EXTENSIONS: [&str; 4] = ["ts", "mts", "js", "mjs"];

pub(crate) struct FsResolver;

impl Resolver for FsResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        // Bare specifiers (`import x from "lodash"`) would need node_modules
        // resolution, which Finicky never offered either.
        if !(name.starts_with("./") || name.starts_with("../") || Path::new(name).is_absolute()) {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "only relative and absolute imports are supported",
            ));
        }
        let candidate = Path::new(base)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(name);
        find(&candidate)
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| rquickjs::Error::new_resolving(base, name))
    }
}

fn find(candidate: &Path) -> Option<PathBuf> {
    if candidate.is_file() {
        return Some(candidate.to_path_buf());
    }
    EXTENSIONS
        .iter()
        .map(|ext| candidate.with_extension(ext))
        .find(|p| p.is_file())
}

/// Loads imports through oxc and records each file, so a reload can watch
/// everything the config was built from.
pub(crate) struct OxcLoader {
    pub loaded: Rc<RefCell<Vec<PathBuf>>>,
}

impl Loader for OxcLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let path = Path::new(name);
        {
            let mut loaded = self.loaded.borrow_mut();
            let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !loaded.contains(&canonical) {
                loaded.push(canonical);
            }
        }
        let source = fs::read_to_string(path)
            .map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;
        let transformed = transform(path, &source)
            .map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;
        Module::declare(ctx.clone(), name, transformed.code)
    }
}
