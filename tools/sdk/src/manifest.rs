//! `mod.toml` (one mod's metadata and options) and `modpack.toml` (a selection of mods with
//! option overrides and change requests). Shared by `lina`, the `openlina` helper and the website.
//!
//! ```toml
//! # mod.toml
//! [mod]
//! id = "screen-wrap"
//! name = "Screen Wrap"
//! version = "0.2.0"
//! kit = "0.1.0"                    # openlina-kit it was made with (see `kit`); `lina pack` keeps it current
//! section = "modifiers"            # items | modifiers | levels | general | core | dev
//! description = "Objects leaving the screen come back on the other side."
//! authors = ["openlina-kit"]
//! game_builds = ["22056877"]       # builds it was tested with (informational)
//! requires = ["core"]              # must be in the pack; applied before this mod
//! after = []                       # applied before this mod if present
//! conflicts = []
//! showcase = ["best.gif"]          # media/ gifs, in the website's order (others follow)
//!
//! [options.coins]
//! type = "bool"                    # bool | int | float | string | list (of strings)
//! default = false
//! description = "Also wrap fruits."
//!
//! [stats]                          # free-form, shown on the website (items: ammo, aim, ...)
//!
//! [[conflict]]                     # options that can't work with another mod's options
//! with = "solid-edges"
//! options = { always = true }      # this mod's options (all must match; none: always)
//! with_options = { always = true } # the other mod's options (all must match; none: always)
//! reason = "the border can't be a portal and a wall in every level"
//! ```
//!
//! ```toml
//! # modpack.toml
//! openlina = 1
//! [[mod]]
//! id = "screen-wrap"
//! options = { coins = true }
//! request = "only wrap the bottom edge"   # change request for an agent (optional)
//! [section_requests]
//! modifiers = "..."
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Core,
    Items,
    Modifiers,
    Levels,
    General,
    Dev,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    #[serde(rename = "mod")]
    pub info: ModInfo,
    #[serde(default)]
    pub options: BTreeMap<String, OptionSpec>,
    #[serde(default)]
    pub stats: toml::Table,
    /// Option combinations that can't work together with another mod (`[[conflict]]`).
    #[serde(default, rename = "conflict", skip_serializing_if = "Vec::is_empty")]
    pub option_conflicts: Vec<OptionConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The openlina-kit version the mod was made with (`crate::kit`); none: before kit versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    pub section: Section,
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub game_builds: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// Showcase gifs in `media/` in the order the website shows them (the first leads); gifs
    /// not listed follow alphabetically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub showcase: Vec<String>,
}

/// A combination of this mod's options and another mod's options that can't work. Checked
/// wherever a pack is put together (builds, `openlina set`, the website), with the reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionConflict {
    pub with: String,
    #[serde(default)]
    pub options: toml::Table,
    #[serde(default)]
    pub with_options: toml::Table,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionType {
    Bool,
    Int,
    Float,
    String,
    /// An array of strings.
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionSpec {
    #[serde(rename = "type")]
    pub kind: OptionType,
    pub default: toml::Value,
    #[serde(default)]
    pub description: String,
}

impl OptionSpec {
    fn check(&self, key: &str, v: &toml::Value) -> Result<toml::Value> {
        Ok(match (self.kind, v) {
            (OptionType::Bool, toml::Value::Boolean(_))
            | (OptionType::Int, toml::Value::Integer(_))
            | (OptionType::Float, toml::Value::Float(_))
            | (OptionType::String, toml::Value::String(_)) => v.clone(),
            (OptionType::List, toml::Value::Array(a)) if a.iter().all(|x| x.is_str()) => v.clone(),
            (OptionType::Float, toml::Value::Integer(i)) => toml::Value::Float(*i as f64),
            _ => bail!("option `{key}` must be {:?} (like its default, {}), got {v}", self.kind, self.default),
        })
    }
}

impl ModManifest {
    pub fn parse(s: &str) -> Result<Self> {
        let m: Self = toml::from_str(s)?;
        m.check()?;
        Ok(m)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&s).with_context(|| format!("in {}", path.display()))
    }

    fn check(&self) -> Result<()> {
        let id = &self.info.id;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            bail!("mod id `{id}` must be lowercase letters, digits and `-`");
        }
        for (k, o) in &self.options {
            o.check(k, &o.default).with_context(|| format!("default of option `{k}`"))?;
        }
        if let Some(k) = &self.info.kit {
            crate::kit::KitVersion::parse(k)?;
        }
        for c in &self.option_conflicts {
            self.resolve_options(&c.options).with_context(|| format!("[[conflict]] with `{}`", c.with))?;
        }
        Ok(())
    }

    /// The kit version the mod was made with ([`crate::kit::UNVERSIONED`] if it doesn't say).
    pub fn kit(&self) -> crate::kit::KitVersion {
        let v = self.info.kit.as_deref().unwrap_or(crate::kit::UNVERSIONED);
        crate::kit::KitVersion::parse(v).unwrap_or(crate::kit::KitVersion(0, 0, 0))
    }

    /// The options to pass to the patch: declared defaults, overridden by `user`.
    pub fn resolve_options(&self, user: &toml::Table) -> Result<toml::Table> {
        let mut out = toml::Table::new();
        for (k, o) in &self.options {
            out.insert(k.clone(), o.default.clone());
        }
        for (k, v) in user {
            let spec = self.options.get(k).with_context(|| format!("mod `{}` has no option `{k}`", self.info.id))?;
            out.insert(k.clone(), spec.check(k, v)?);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModPack {
    #[serde(default = "one")]
    pub openlina: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_build: Option<String>,
    #[serde(default, rename = "mod")]
    pub mods: Vec<PackEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub section_requests: BTreeMap<String, String>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub options: toml::Table,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Review status on the site it came from (`reviewed`, `unreviewed`); none for local builds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

impl ModPack {
    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let p: Self = toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        if p.openlina != 1 {
            bail!("{}: unsupported modpack version {}", path.display(), p.openlina);
        }
        Ok(p)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }
}

/// What every mod gets to know about the pack it is built into (`OPENLINA_PACK`, TOML), e.g. for
/// a mod menu: the mods in application order with their resolved options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackInfo {
    #[serde(default, rename = "mod")]
    pub mods: Vec<PackInfoEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInfoEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub section: Section,
    #[serde(default)]
    pub options: toml::Table,
}

/// The declared option conflicts (`[[conflict]]`) that hold between these mods with these resolved
/// options, as messages: "solid-edges (always = true) and screen-wrap (always = true): <reason>".
pub fn option_conflicts(mods: &[(&ModManifest, &toml::Table)]) -> Vec<String> {
    let matches = |want: &toml::Table, have: &toml::Table| {
        want.iter().all(|(k, v)| match (have.get(k), v) {
            (Some(toml::Value::Float(h)), toml::Value::Integer(w)) => *h == *w as f64,
            (Some(h), w) => h == w,
            (None, _) => false,
        })
    };
    let show = |id: &str, t: &toml::Table| {
        if t.is_empty() {
            id.to_string()
        } else {
            let parts: Vec<String> = t.iter().map(|(k, v)| format!("{k} = {v}")).collect();
            format!("{id} ({})", parts.join(", "))
        }
    };
    let mut out = Vec::new();
    for (m, opts) in mods {
        for c in &m.option_conflicts {
            let Some((_, other)) = mods.iter().find(|(o, _)| o.info.id == c.with) else { continue };
            if matches(&c.options, opts) && matches(&c.with_options, other) {
                out.push(format!(
                    "{} and {}: {}",
                    show(&m.info.id, &c.options),
                    show(&c.with, &c.with_options),
                    c.reason
                ));
            }
        }
    }
    out
}

/// Order in which to apply mods: every `requires`/`after` dependency first, `core` section
/// first among equals, then by id. Fails on missing requirements, conflicts and cycles.
pub fn resolve_order(mods: &[ModManifest]) -> Result<Vec<usize>> {
    let index: BTreeMap<&str, usize> = mods.iter().enumerate().map(|(i, m)| (m.info.id.as_str(), i)).collect();
    if index.len() != mods.len() {
        bail!("the same mod appears twice");
    }
    for m in mods {
        for r in &m.info.requires {
            if !index.contains_key(r.as_str()) {
                bail!("`{}` requires `{r}`, which is not in the pack", m.info.id);
            }
        }
        for c in &m.info.conflicts {
            if index.contains_key(c.as_str()) {
                bail!(
                    "`{}` conflicts with `{c}` (its mod.toml says so): remove one, or change them to work together \
                     (AGENTS.md, \"Load order\")",
                    m.info.id
                );
            }
        }
    }
    let deps = |m: &ModManifest| -> Vec<usize> {
        m.info.requires.iter().chain(&m.info.after).filter_map(|d| index.get(d.as_str()).copied()).collect()
    };
    let mut done = BTreeSet::new();
    let mut order = Vec::new();
    while order.len() < mods.len() {
        let mut ready: Vec<usize> =
            (0..mods.len()).filter(|i| !done.contains(i) && deps(&mods[*i]).iter().all(|d| done.contains(d))).collect();
        if ready.is_empty() {
            let stuck: Vec<_> =
                (0..mods.len()).filter(|i| !done.contains(i)).map(|i| mods[i].info.id.clone()).collect();
            bail!("dependency cycle between {}", stuck.join(", "));
        }
        ready.sort_by_key(|&i| (mods[i].info.section != Section::Core, mods[i].info.id.clone()));
        let next = ready[0];
        done.insert(next);
        order.push(next);
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, section: &str, requires: &[&str], after: &[&str]) -> ModManifest {
        ModManifest::parse(&format!(
            "[mod]\nid = \"{id}\"\nname = \"x\"\nversion = \"0.1.0\"\nsection = \"{section}\"\ndescription = \"\"\n\
             requires = {requires:?}\nafter = {after:?}\n\n[options.n]\ntype = \"float\"\ndefault = 1.0\n"
        ))
        .unwrap()
    }

    fn ids(mods: &[ModManifest]) -> Vec<String> {
        resolve_order(mods).unwrap().into_iter().map(|i| mods[i].info.id.clone()).collect()
    }

    #[test]
    fn order_puts_dependencies_and_core_first() {
        let mods = [
            m("zeta", "items", &["core"], &["alpha"]),
            m("alpha", "items", &["core"], &[]),
            m("core", "core", &[], &[]),
        ];
        assert_eq!(ids(&mods), ["core", "alpha", "zeta"]);
    }

    #[test]
    fn order_reports_missing_requirements_and_cycles() {
        assert!(resolve_order(&[m("a", "items", &["core"], &[])]).is_err());
        assert!(resolve_order(&[m("a", "items", &[], &["b"]), m("b", "items", &[], &["a"])]).is_err());
    }

    #[test]
    fn option_conflicts_need_all_options_to_match() {
        let a = ModManifest::parse(
            "[mod]\nid = \"a\"\nname = \"x\"\nversion = \"0.1.0\"\nsection = \"items\"\ndescription = \"\"\n\
             [options.always]\ntype = \"bool\"\ndefault = false\n\
             [[conflict]]\nwith = \"b\"\noptions = { always = true }\nwith_options = { n = 2.0 }\nreason = \"no\"\n",
        )
        .unwrap();
        let b = m("b", "items", &[], &[]);
        let opts = |m: &ModManifest, kv: &str| m.resolve_options(&toml::from_str(kv).unwrap()).unwrap();
        let (on, off) = (opts(&a, "always = true"), opts(&a, ""));
        let (two, one) = (opts(&b, "n = 2"), opts(&b, ""));
        assert_eq!(option_conflicts(&[(&a, &on), (&b, &two)]).len(), 1);
        assert!(option_conflicts(&[(&a, &off), (&b, &two)]).is_empty());
        assert!(option_conflicts(&[(&a, &on), (&b, &one)]).is_empty());
        assert!(option_conflicts(&[(&a, &on)]).is_empty());
        // options must exist in the declaring mod
        assert!(ModManifest::parse(
            "[mod]\nid = \"a\"\nname = \"x\"\nversion = \"0.1.0\"\nsection = \"items\"\ndescription = \"\"\n\
             [[conflict]]\nwith = \"b\"\noptions = { nope = true }\nreason = \"no\"\n"
        )
        .is_err());
    }

    #[test]
    fn options_are_checked_and_defaulted() {
        let x = m("a", "items", &[], &[]);
        assert_eq!(x.resolve_options(&toml::Table::new()).unwrap()["n"], toml::Value::Float(1.0));
        let mut t = toml::Table::new();
        t.insert("n".into(), toml::Value::Integer(3));
        assert_eq!(x.resolve_options(&t).unwrap()["n"], toml::Value::Float(3.0));
        t.insert("n".into(), toml::Value::String("no".into()));
        assert!(x.resolve_options(&t).is_err());
        let mut u = toml::Table::new();
        u.insert("nope".into(), toml::Value::Boolean(true));
        assert!(x.resolve_options(&u).is_err());
    }
}
