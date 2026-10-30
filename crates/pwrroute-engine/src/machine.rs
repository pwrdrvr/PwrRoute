//! The machine layer: what synced rules mean on this machine.
//!
//! `rules.json` can sync between machines (the app's sync service copies it
//! to and from a cloud folder). What differs between machines lives beside
//! it in `machine.json`, which never syncs:
//!
//! ```json
//! {
//!   "name": "Studio Mac",
//!   "destinations": { "Work 1": { "browser": "Google Chrome", "profile": "Profile 3" } },
//!   "sets": { "Work 2": false }
//! }
//! ```
//!
//! - `destinations` binds a named destination (`rules.json`'s
//!   `destinations[]`, named by `rules[].destination` and
//!   `defaultDestination`) to a browser and profile here. It wins over the
//!   synced binding; a name with no binding here uses the synced one.
//! - `sets` turns a rule set (`rules[].set`, `rewrites[].set`) on or off
//!   here. It wins over `sets[].enabled`; a set neither names is on.
//! - `name` is this machine's name in sync status.
//!
//! [`RulesFile::for_machine`] applies the layer: each entry naming a
//! destination gets its browser and profile, each entry in a set that is
//! off is disabled (it keeps its place, so "rule N" keeps one meaning), and
//! `defaultDestination` sets `defaultBrowser`/`defaultProfile`. Every engine
//! applies the synced layer itself ([`RulesFile::resolved`]); whoever loads
//! the files for routing applies this machine's ([`MachineFile::load_beside`]).

use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::rules::{Binding, Destination, RuleSet, RulesFile};

/// `machine.json`: this machine's bindings and rule-set switches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineFile {
    /// This machine's name in sync status; empty means the computer's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub destinations: BTreeMap<String, Binding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sets: BTreeMap<String, bool>,
}

/// Where a destination's meaning on this machine comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSource {
    /// `machine.json` binds it here.
    Machine,
    /// The synced binding in `rules.json`.
    Synced,
}

impl MachineFile {
    pub const FILE_NAME: &'static str = "machine.json";

    /// `machine.json` beside a `rules.json`.
    pub fn path_beside(rules_path: &Path) -> PathBuf {
        rules_path.with_file_name(Self::FILE_NAME)
    }

    /// Reads `machine.json`. A missing file is an empty layer.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("{}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("{}", path.display())),
        }
    }

    /// The layer beside `rules_path`, or an empty one when it is missing or
    /// does not parse: a broken `machine.json` must not stop the synced
    /// rules from routing. [`MachineFile::load`] reports the error.
    pub fn load_beside(rules_path: &Path) -> Self {
        Self::load(&Self::path_beside(rules_path)).unwrap_or_default()
    }

    /// Writes `machine.json` through a sibling file renamed into place.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("{}", dir.display()))?;
        }
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        let mut staging = path.as_os_str().to_owned();
        staging.push(".saving");
        fs::write(&staging, text).with_context(|| format!("{}", path.display()))?;
        fs::rename(&staging, path).with_context(|| format!("{}", path.display()))
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.routes_as_synced()
    }

    /// Whether the layer changes nothing about routing (it may still name
    /// the machine).
    pub fn routes_as_synced(&self) -> bool {
        self.destinations.is_empty() && self.sets.is_empty()
    }
}

impl RulesFile {
    /// The synced binding of a named destination.
    pub fn destination(&self, name: &str) -> Option<&Binding> {
        self.destinations
            .iter()
            .find(|d| d.name == name)
            .map(|d| &d.binding)
    }

    /// What a named destination opens on a machine, and where that comes
    /// from; `None` when neither layer binds the name.
    pub fn binding_for<'a>(
        &'a self,
        name: &str,
        machine: &'a MachineFile,
    ) -> Option<(&'a Binding, BindingSource)> {
        machine
            .destinations
            .get(name)
            .map(|b| (b, BindingSource::Machine))
            .or_else(|| self.destination(name).map(|b| (b, BindingSource::Synced)))
    }

    /// Whether a rule set is on by default (before any machine's switch).
    pub fn set_enabled(&self, name: &str) -> bool {
        self.sets
            .iter()
            .find(|s| s.name == name)
            .is_none_or(|s| s.enabled)
    }

    /// Whether a rule set is on for a machine: its switch there, else the
    /// synced default.
    pub fn set_enabled_on(&self, name: &str, machine: &MachineFile) -> bool {
        machine
            .sets
            .get(name)
            .copied()
            .unwrap_or_else(|| self.set_enabled(name))
    }

    /// Every set an entry or `sets` names, in first-seen order.
    pub fn set_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let named = self.sets.iter().map(|s| s.name.as_str());
        let used = self
            .rewrites
            .iter()
            .filter_map(|r| r.set.as_deref())
            .chain(self.rules.iter().filter_map(|r| r.set.as_deref()));
        for name in named.chain(used) {
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
        names
    }

    /// The file as the synced layer alone resolves it: [`RulesFile::for_machine`]
    /// with no machine layer. Every engine loads this, so a hand-edited
    /// `browser` that disagrees with its destination's binding loses.
    pub fn resolved(&self) -> RulesFile {
        self.for_machine(&MachineFile::default())
    }

    /// The file as `machine` routes it: each entry that names a destination
    /// opens what the destination means there, each entry in a set that is
    /// off there is disabled in place, and `defaultDestination` sets the
    /// default browser. The bindings and switches used are written into
    /// `destinations` and `sets`, so applying it again changes nothing.
    /// Never save the result as the synced file.
    pub fn for_machine(&self, machine: &MachineFile) -> RulesFile {
        let mut out = self.clone();
        for destination in &mut out.destinations {
            if let Some(binding) = machine.destinations.get(&destination.name) {
                destination.binding = binding.clone();
            }
        }
        // A name bound here that the synced file no longer defines still
        // routes as bound, as `binding_for` (what the editor shows) says.
        for (name, binding) in &machine.destinations {
            if out.destination(name).is_none() {
                out.destinations.push(Destination {
                    name: name.clone(),
                    binding: binding.clone(),
                });
            }
        }
        for (name, on) in &machine.sets {
            match out.sets.iter_mut().find(|s| &s.name == name) {
                Some(set) => set.enabled = *on,
                None => out.sets.push(RuleSet {
                    name: name.clone(),
                    enabled: *on,
                }),
            }
        }
        let bindings: HashMap<&str, Binding> = out
            .destinations
            .iter()
            .filter(|d| !d.binding.browser.trim().is_empty())
            .map(|d| (d.name.as_str(), d.binding.clone()))
            .collect();
        let off: Vec<String> = out
            .sets
            .iter()
            .filter(|s| !s.enabled)
            .map(|s| s.name.clone())
            .collect();
        let is_off = |set: &Option<String>| set.as_ref().is_some_and(|s| off.contains(s));
        for rule in &mut out.rules {
            if let Some(binding) = rule.destination.as_deref().and_then(|n| bindings.get(n)) {
                rule.browser.clone_from(&binding.browser);
                rule.profile.clone_from(&binding.profile);
                rule.private |= binding.private;
            }
            if is_off(&rule.set) {
                rule.enabled = false;
            }
        }
        for rewrite in &mut out.rewrites {
            if is_off(&rewrite.set) {
                rewrite.enabled = false;
            }
        }
        if let Some(binding) = out
            .default_destination
            .as_deref()
            .and_then(|n| bindings.get(n))
        {
            out.default_browser.clone_from(&binding.browser);
            out.default_profile.clone_from(&binding.profile);
        }
        out
    }

    /// Copies each named destination's synced binding into the entries that
    /// name it (`browser`, `profile`, and the default's), so a reader that
    /// ignores names (the Go host) routes the way the synced layer does.
    /// The editor calls it after changing a destination or a rule's name.
    pub fn mirror_destinations(&mut self) {
        let synced: HashMap<String, Binding> = self
            .destinations
            .iter()
            .filter(|d| !d.binding.browser.trim().is_empty())
            .map(|d| (d.name.clone(), d.binding.clone()))
            .collect();
        for rule in &mut self.rules {
            if let Some(binding) = rule.destination.as_ref().and_then(|n| synced.get(n)) {
                rule.browser.clone_from(&binding.browser);
                rule.profile.clone_from(&binding.profile);
            }
        }
        if let Some(binding) = self
            .default_destination
            .as_ref()
            .and_then(|n| synced.get(n))
        {
            self.default_browser.clone_from(&binding.browser);
            self.default_profile.clone_from(&binding.profile);
        }
    }

    /// How many rules (handlers) name each destination, by name.
    pub fn destination_uses(&self, name: &str) -> usize {
        self.rules
            .iter()
            .filter(|r| r.destination.as_deref() == Some(name))
            .count()
            + usize::from(self.default_destination.as_deref() == Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Rewrite, Rule};

    fn binding(browser: &str, profile: &str) -> Binding {
        Binding {
            browser: browser.into(),
            profile: profile.into(),
            private: false,
        }
    }

    /// The user's case: four named targets, the same on most machines.
    fn synced() -> RulesFile {
        RulesFile {
            default_browser: "Google Chrome".into(),
            default_profile: "Personal".into(),
            default_destination: Some("Personal".into()),
            destinations: vec![
                Destination {
                    name: "Work 1".into(),
                    binding: binding("Google Chrome", "Work"),
                },
                Destination {
                    name: "Work 2".into(),
                    binding: binding("Microsoft Edge", "Client"),
                },
                Destination {
                    name: "Personal".into(),
                    binding: binding("Google Chrome", "Personal"),
                },
                Destination {
                    name: "PwrDrvr".into(),
                    binding: binding("Arc", ""),
                },
            ],
            sets: vec![RuleSet {
                name: "Client".into(),
                enabled: true,
            }],
            rules: vec![
                Rule {
                    patterns: vec!["work.example.com/*".into()],
                    browser: "Google Chrome".into(),
                    profile: "Work".into(),
                    destination: Some("Work 1".into()),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["client.example.net/*".into()],
                    browser: "Microsoft Edge".into(),
                    profile: "Client".into(),
                    destination: Some("Work 2".into()),
                    set: Some("Client".into()),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["pwrdrvr.example/*".into()],
                    browser: "Arc".into(),
                    destination: Some("PwrDrvr".into()),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["plain.example/*".into()],
                    browser: "Safari".into(),
                    ..Rule::default()
                },
            ],
            rewrites: vec![Rewrite {
                patterns: vec!["client.example.net/*".into()],
                actions: vec![crate::RewriteAction::SetScheme("https".into())],
                set: Some("Client".into()),
                ..Rewrite::default()
            }],
            ..RulesFile::default()
        }
    }

    #[test]
    fn with_no_machine_layer_the_synced_bindings_route() {
        let file = synced();
        assert_eq!(
            file.resolved(),
            file,
            "already mirrored, so nothing changes"
        );
    }

    #[test]
    fn a_machine_binding_wins_and_unbound_names_keep_the_synced_one() {
        let machine = MachineFile {
            destinations: BTreeMap::from([
                ("PwrDrvr".into(), binding("Google Chrome", "Profile 3")),
                ("Personal".into(), binding("Firefox", "default-release")),
            ]),
            ..MachineFile::default()
        };
        let here = synced().for_machine(&machine);
        assert_eq!(
            (
                here.rules[2].browser.as_str(),
                here.rules[2].profile.as_str()
            ),
            ("Google Chrome", "Profile 3")
        );
        assert_eq!(
            (
                here.rules[0].browser.as_str(),
                here.rules[0].profile.as_str()
            ),
            ("Google Chrome", "Work")
        );
        assert_eq!(
            here.rules[3].browser, "Safari",
            "no destination: as written"
        );
        assert_eq!(
            (here.default_browser.as_str(), here.default_profile.as_str()),
            ("Firefox", "default-release")
        );
        assert_eq!(
            here.for_machine(&MachineFile::default()),
            here,
            "idempotent"
        );
        assert_eq!(here.resolved(), here);
    }

    #[test]
    fn binding_for_says_where_the_meaning_comes_from() {
        let file = synced();
        let machine = MachineFile {
            destinations: BTreeMap::from([("PwrDrvr".into(), binding("Firefox", ""))]),
            ..MachineFile::default()
        };
        assert_eq!(
            file.binding_for("PwrDrvr", &machine)
                .map(|(b, s)| (b.browser.as_str(), s)),
            Some(("Firefox", BindingSource::Machine))
        );
        assert_eq!(
            file.binding_for("Work 1", &machine).map(|(_, s)| s),
            Some(BindingSource::Synced)
        );
        assert!(file.binding_for("Nowhere", &machine).is_none());
    }

    #[test]
    fn a_set_off_on_this_machine_disables_its_entries_in_place() {
        let machine = MachineFile {
            sets: BTreeMap::from([("Client".into(), false)]),
            ..MachineFile::default()
        };
        let here = synced().for_machine(&machine);
        assert_eq!(here.rules.len(), 4, "rule numbers keep their meaning");
        assert!(!here.rules[1].enabled);
        assert!(!here.rewrites[0].enabled);
        assert!(here.rules[0].enabled && here.rules[2].enabled);
        assert_eq!(here.planned_rules(), vec![0, 2, 3]);
        assert_eq!(here.planned_rewrites(), Vec::<usize>::new());
    }

    #[test]
    fn a_set_off_everywhere_can_be_turned_on_here() {
        let mut file = synced();
        file.sets[0].enabled = false;
        assert!(!file.resolved().rules[1].enabled);
        let machine = MachineFile {
            sets: BTreeMap::from([("Client".into(), true)]),
            ..MachineFile::default()
        };
        assert!(file.for_machine(&machine).rules[1].enabled);
        assert!(file.set_enabled_on("Client", &machine));
        assert!(!file.set_enabled("Client"));
        // A set only the machine names is recorded, and the rest are on.
        assert!(file.set_enabled_on("Other", &MachineFile::default()));
    }

    #[test]
    fn an_undefined_destination_falls_back_to_the_rules_own_browser() {
        let mut file = synced();
        file.rules[0].destination = Some("Gone".into());
        let here = file.resolved();
        assert_eq!(here.rules[0].browser, "Google Chrome");
        assert_eq!(here.rules[0].profile, "Work");
    }

    #[test]
    fn a_name_bound_here_that_the_file_lost_still_routes_as_bound() {
        let mut file = synced();
        file.destinations.retain(|d| d.name != "Work 1");
        let machine = MachineFile {
            destinations: BTreeMap::from([(
                "Work 1".to_owned(),
                binding("Microsoft Edge", "Profile 3"),
            )]),
            ..MachineFile::default()
        };
        let (shown, _) = file.binding_for("Work 1", &machine).unwrap();
        let here = file.for_machine(&machine);
        assert_eq!(here.rules[0].browser, shown.browser);
        assert_eq!(here.rules[0].profile, "Profile 3");
        assert_eq!(here.for_machine(&machine), here, "idempotent");
    }

    #[test]
    fn a_private_binding_opens_a_private_window() {
        let machine = MachineFile {
            destinations: BTreeMap::from([(
                "Work 1".into(),
                Binding {
                    private: true,
                    ..binding("Google Chrome", "Work")
                },
            )]),
            ..MachineFile::default()
        };
        assert!(synced().for_machine(&machine).rules[0].private);
        assert!(!synced().resolved().rules[0].private);
    }

    #[test]
    fn mirroring_copies_the_synced_binding_for_name_blind_readers() {
        let mut file = synced();
        file.destinations[0].binding = binding("Firefox", "work");
        file.mirror_destinations();
        assert_eq!(file.rules[0].browser, "Firefox");
        assert_eq!(file.rules[0].profile, "work");
        assert_eq!(file.rules[3].browser, "Safari");
        file.destinations[2].binding = binding("Safari", "");
        file.mirror_destinations();
        assert_eq!(file.default_browser, "Safari");
        assert_eq!(file.default_profile, "");
    }

    #[test]
    fn set_names_and_uses() {
        let file = synced();
        assert_eq!(file.set_names(), vec!["Client".to_owned()]);
        assert_eq!(file.destination_uses("Personal"), 1);
        assert_eq!(file.destination_uses("Work 2"), 1);
    }

    #[test]
    fn machine_file_round_trips_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let rules = dir.path().join("rules.json");
        let path = MachineFile::path_beside(&rules);
        assert_eq!(path, dir.path().join("machine.json"));
        assert!(MachineFile::load(&path).unwrap().is_empty());
        let machine = MachineFile {
            name: "Studio Mac".into(),
            destinations: BTreeMap::from([("Work 1".into(), binding("Firefox", "work"))]),
            sets: BTreeMap::from([("Client".into(), false)]),
        };
        machine.save(&path).unwrap();
        assert_eq!(MachineFile::load(&path).unwrap(), machine);
        assert_eq!(MachineFile::load_beside(&rules), machine);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"Work 1\": {"), "{text}");
        std::fs::write(&path, "{ nope").unwrap();
        assert!(MachineFile::load(&path).is_err());
        assert!(
            MachineFile::load_beside(&rules).is_empty(),
            "broken: synced alone"
        );
    }
}
