//! `mosa mods`, `mosa build`, `mosa check`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use mosa_bc::{validate, Code, ModConfig};

use crate::game;

pub fn list_mods() -> Result<()> {
    for m in mosa_mods::all() {
        println!("{}\n    {}", m.id(), m.description());
        for (k, default, desc) in m.options() {
            println!("    - {k} (default {default}): {desc}");
        }
    }
    Ok(())
}

pub fn build(config: &Path, only: &[String], out: &Path) -> Result<()> {
    let hash = game::sha256(Path::new(game::ORIG)).context("run `mosa setup` first")?;
    if game::known_version(&hash).is_none() {
        eprintln!("WARNING: {} is not a known game version ({hash})", game::ORIG);
    }
    let mut code = Code::load(game::ORIG)?;

    let cfg: toml::Table = match std::fs::read_to_string(config) {
        Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", config.display()))?,
        Err(_) if !only.is_empty() => toml::Table::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", config.display())),
    };
    let mods = mosa_mods::all();
    for key in cfg.keys() {
        if !mods.iter().any(|m| m.id() == key) {
            bail!("{}: unknown mod `{key}` (see `mosa mods`)", config.display());
        }
    }
    for id in only {
        if !mods.iter().any(|m| m.id() == id) {
            bail!("unknown mod `{id}` (see `mosa mods`)");
        }
    }

    let mut applied = Vec::new();
    for m in &mods {
        let mut table = match cfg.get(m.id()) {
            Some(toml::Value::Table(t)) => t.clone(),
            Some(_) => bail!("`{}` in {} must be a table", m.id(), config.display()),
            None => toml::Table::new(),
        };
        let enabled = if only.is_empty() {
            match table.remove("enabled") {
                Some(toml::Value::Boolean(b)) => b,
                Some(_) => bail!("`{}.enabled` must be a boolean", m.id()),
                None => cfg.contains_key(m.id()),
            }
        } else {
            table.remove("enabled");
            only.iter().any(|o| o == m.id())
        };
        if !enabled {
            continue;
        }
        for key in table.keys() {
            if !m.options().iter().any(|(k, _, _)| k == key) {
                bail!("mod `{}` has no option `{key}` (see `mosa mods`)", m.id());
            }
        }
        m.apply(&mut code, &ModConfig { table }).with_context(|| format!("applying mod `{}`", m.id()))?;
        applied.push(m.id());
    }

    validate::check_touched(&code)?;
    let touched = code.touched.clone();
    // Round-trip through the serializer so that what we validate is exactly what we ship.
    let bytes = code.to_bytes()?;
    let mut reparsed = Code::from_bytes(&bytes).context("re-parsing the patched bytecode")?;
    reparsed.touched = touched.clone();
    validate::check_touched(&reparsed).context("after re-parse")?;

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, &bytes)?;
    println!(
        "applied {} mod(s) [{}], {} function(s) patched or added -> {}",
        applied.len(),
        applied.join(", "),
        touched.len(),
        out.display()
    );
    for f in touched {
        println!("  fn@{} {}", f.0, reparsed.func_name(f));
    }
    Ok(())
}

pub fn check(input: &Path) -> Result<()> {
    let code = Code::load(input)?;
    let bytes = code.to_bytes()?;
    let again = Code::from_bytes(&bytes)?;
    if again.bc.functions.len() != code.bc.functions.len()
        || again.bc.types.len() != code.bc.types.len()
        || again.to_bytes()?.len() != bytes.len()
    {
        bail!("serializer roundtrip changed the bytecode");
    }
    println!("roundtrip ok ({} bytes)", bytes.len());

    let mut problems = 0;
    let mut shown = 0;
    for f in &code.bc.functions {
        let errs = validate::check_function(&code, f);
        problems += errs.len();
        for e in errs {
            if shown < 20 {
                println!("  {e}");
                shown += 1;
            }
        }
    }
    println!(
        "validator: {problems} problem(s) in {} vanilla functions (should be 0; anything else is a validator false positive)",
        code.bc.functions.len()
    );
    Ok(())
}
