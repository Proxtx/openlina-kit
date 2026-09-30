//! `lina docs`: the option tables in docs/mods.md, generated from each mod's `mod.toml` (the one
//! source of truth: the website shows the same descriptions). A table lives between
//! `<!-- options:<id> -->` and `<!-- /options -->`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openlina_sdk::manifest::ModManifest;

const DOC: &str = "docs/mods.md";

/// The markdown table of a mod's options, in `mod.toml` order.
fn table(id: &str) -> Result<String> {
    let path = Path::new("mods").join(id).join("mod.toml");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let m = ModManifest::parse(&text)?;
    if m.options.is_empty() {
        return Ok("No options.\n".into());
    }
    // File order (the manifest's map is sorted).
    let order: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("[options.")?.strip_suffix(']').map(str::to_string))
        .collect();
    let mut out = String::from("| option | type | default | description |\n|---|---|---|---|\n");
    for name in &order {
        let Some(o) = m.options.get(name) else { continue };
        let default = match &o.default {
            toml::Value::String(s) => format!("{s:?}"),
            v => v.to_string(),
        };
        let kind = format!("{:?}", o.kind).to_lowercase();
        out.push_str(&format!("| `{name}` | {kind} | `{default}` | {} |\n", o.description.replace('|', "\\|")));
    }
    Ok(out)
}

/// Regenerate every table (or with `check`, fail when one is out of date).
pub fn docs(check: bool) -> Result<()> {
    let text = std::fs::read_to_string(DOC)?;
    let mut out = String::new();
    let mut rest = text.as_str();
    let mut stale = Vec::new();
    let mut count = 0;
    while let Some(p) = rest.find("<!-- options:") {
        let (before, after) = rest.split_at(p);
        out.push_str(before);
        let head_end = after.find("-->").context("unclosed `<!-- options:` marker")? + 3;
        let id = after["<!-- options:".len()..head_end - 3].trim().to_string();
        let close = after.find("<!-- /options -->").with_context(|| format!("no `<!-- /options -->` after {id}"))?;
        let old = &after[head_end..close];
        let new = format!("\n{}", table(&id)?);
        if old != new {
            stale.push(id.clone());
        }
        out.push_str(&after[..head_end]);
        out.push_str(&new);
        out.push_str("<!-- /options -->");
        rest = &after[close + "<!-- /options -->".len()..];
        count += 1;
    }
    out.push_str(rest);
    if check {
        if !stale.is_empty() {
            bail!("{DOC}: option tables out of date for {} (run `lina docs`)", stale.join(", "));
        }
        println!("{count} option tables up to date");
        return Ok(());
    }
    std::fs::write(DOC, out)?;
    println!("{count} option tables written ({} changed)", stale.len());
    Ok(())
}
