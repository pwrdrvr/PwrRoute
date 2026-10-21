//! A fixture machine in a temporary directory.

#![allow(dead_code)] // each test binary uses a different part

use std::{
    fs,
    path::{Path, PathBuf},
};

use pwrroute_browsers::{Env, Platform};
use pwrroute_engine::{AppType, Browser};

pub const LOCAL_STATE: &str = include_str!("../fixtures/chromium-local-state.json");
pub const PROFILES_INI: &str = include_str!("../fixtures/firefox-profiles.ini");
pub const URL: &str = "https://example.com/path?q=a%20b";

pub struct Fixture {
    dir: tempfile::TempDir,
    pub env: Env,
}

impl Fixture {
    pub fn new(platform: Platform) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::fixture(platform, dir.path());
        Self { dir, env }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Writes `contents` at `rel` under the root, creating directories.
    pub fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    /// An absolute path inside the root, as the code under test builds it.
    pub fn path(&self, rel: &str) -> PathBuf {
        let mut p = self.root().to_path_buf();
        for part in rel.split('/') {
            p.push(part);
        }
        p
    }
}

pub fn browser(name: &str, app_type: AppType) -> Browser {
    Browser {
        name: name.into(),
        app_type,
        url: URL.into(),
        profile: String::new(),
        args: Vec::new(),
        open_in_background: None,
        private: false,
        privacy: pwrroute_engine::Privacy::Full,
    }
}

pub fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}
