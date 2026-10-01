//! Kit versions: which openlina-kit a mod was made with, and which mods can run together.
//!
//! The kit (SDK, `lina`, `openlina`, the `core` mod) has one version, `KIT_VERSION`, the workspace
//! version in `Cargo.toml`. A mod's `mod.toml` names the kit version it was built with
//! (`kit = "0.1.0"`; `lina new` writes it, `lina pack` raises it to the kit that built the
//! package). Mods without `kit` predate kit versions and count as [`UNVERSIONED`].
//!
//! Versions are `major.minor.patch`; a **line** is the part that may break mods: the minor while
//! the major is 0 (`0.1.x`), else the major (`1.x`), like Cargo's `^`. Within a line, newer kits run
//! everything older ones made. A new line changed what mods rely on (the SDK's API, the runner's
//! interface, core hooks): mods made for an older line need an agent to port them (CHANGELOG.md
//! says what changed), and mods made with a newer kit need an update of the tool.

use std::fmt;

use anyhow::{bail, Result};

use crate::manifest::ModManifest;

/// This kit's version.
pub const KIT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The kit version of mods whose `mod.toml` has no `kit` (made before kit versions existed).
pub const UNVERSIONED: &str = "0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KitVersion(pub u64, pub u64, pub u64);

impl KitVersion {
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.trim().split('.').collect();
        let n = |p: &str| p.parse::<u64>().ok().filter(|_| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        match parts.as_slice() {
            [a, b, c] => match (n(a), n(b), n(c)) {
                (Some(a), Some(b), Some(c)) => Ok(Self(a, b, c)),
                _ => bail!("kit version `{s}` must be major.minor.patch, e.g. `{KIT_VERSION}`"),
            },
            _ => bail!("kit version `{s}` must be major.minor.patch, e.g. `{KIT_VERSION}`"),
        }
    }

    /// This kit's version.
    pub fn current() -> Self {
        Self::parse(KIT_VERSION).expect("the workspace version is major.minor.patch")
    }

    /// The compatibility line: `(0, minor)` for 0.x, `(major, 0)` from 1.0 on.
    pub fn line(self) -> (u64, u64) {
        if self.0 == 0 {
            (0, self.1)
        } else {
            (self.0, 0)
        }
    }

    /// The line as people write it: `0.1` or `1`.
    pub fn line_name(self) -> String {
        match self.line() {
            (0, m) => format!("0.{m}"),
            (a, _) => a.to_string(),
        }
    }
}

impl fmt::Display for KitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// What keeps a mod from running with a tool of kit version `host`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KitIssue {
    /// Made for an older line: an agent ports it to the host's line.
    Port { id: String, version: String, kit: KitVersion },
    /// Made with a newer kit than the host: update the host (the helper, the kit checkout, the site).
    Update { id: String, version: String, kit: KitVersion },
}

impl KitIssue {
    pub fn id(&self) -> &str {
        match self {
            Self::Port { id, .. } | Self::Update { id, .. } => id,
        }
    }

    /// "swap 0.2.0 (made with openlina-kit 0.1.3)"
    pub fn describe(&self) -> String {
        match self {
            Self::Port { id, version, kit } | Self::Update { id, version, kit } => {
                format!("{id} {version} (made with openlina-kit {kit})")
            }
        }
    }
}

/// The issue, if any, of running `m` with a tool of kit version `host`.
pub fn issue(m: &ModManifest, host: KitVersion) -> Option<KitIssue> {
    let kit = m.kit();
    let (id, version) = (m.info.id.clone(), m.info.version.clone());
    if kit.line() < host.line() {
        Some(KitIssue::Port { id, version, kit })
    } else if kit > host {
        Some(KitIssue::Update { id, version, kit })
    } else {
        None
    }
}

/// The issues of running all of `mods` with a tool of kit version `host`.
pub fn issues<'a>(mods: impl IntoIterator<Item = &'a ModManifest>, host: KitVersion) -> Vec<KitIssue> {
    mods.into_iter().filter_map(|m| issue(m, host)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(kit: Option<&str>) -> ModManifest {
        let kit = kit.map(|k| format!("kit = \"{k}\"\n")).unwrap_or_default();
        ModManifest::parse(&format!(
            "[mod]\nid = \"a\"\nname = \"A\"\nversion = \"1.0.0\"\nsection = \"items\"\ndescription = \"\"\n{kit}"
        ))
        .unwrap()
    }

    #[test]
    fn versions_parse_and_form_lines() {
        let v = KitVersion::parse("0.3.12").unwrap();
        assert_eq!(v, KitVersion(0, 3, 12));
        assert_eq!(v.line(), (0, 3));
        assert_eq!(KitVersion(2, 4, 1).line(), (2, 0));
        assert_eq!(KitVersion(0, 3, 1).line_name(), "0.3");
        for bad in ["0.3", "0.3.x", "v0.3.1", "0.3.1.2", "", "0..1"] {
            assert!(KitVersion::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn issues_follow_the_line() {
        let host = KitVersion(0, 2, 3);
        assert_eq!(issue(&m(Some("0.2.0")), host), None);
        assert_eq!(issue(&m(Some("0.2.3")), host), None);
        assert!(matches!(issue(&m(Some("0.2.4")), host), Some(KitIssue::Update { .. })));
        assert!(matches!(issue(&m(Some("0.3.0")), host), Some(KitIssue::Update { .. })));
        assert!(matches!(issue(&m(Some("0.1.9")), host), Some(KitIssue::Port { .. })));
        // no `kit`: made before versions, i.e. 0.1.0
        assert!(matches!(issue(&m(None), host), Some(KitIssue::Port { .. })));
        assert_eq!(issue(&m(None), KitVersion(0, 1, 5)), None);
    }

    #[test]
    fn bad_kit_fields_are_refused() {
        let s = "[mod]\nid = \"a\"\nname = \"A\"\nversion = \"1.0.0\"\nsection = \"items\"\ndescription = \"\"\nkit = \"1\"\n";
        assert!(ModManifest::parse(s).is_err());
    }
}
