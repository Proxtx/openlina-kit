//! Talking to an OpenLina site (openlina-web): `lina login`, `lina pull`, `lina publish`.
//!
//! - `login <site> <token>` checks the token (`GET /api/me`) and saves both in the user's config
//!   (`$OPENLINA_CONFIG`, else `~/.config/openlina/lina.toml`, mode 600).
//! - `pull <pack>` takes a pack link (`https://site/api/packs/<id>`) or id, downloads every package,
//!   puts the source of mods you don't have into `mods/<id>/`, and writes `work/pull/<pack>/`:
//!   `pack.json`, `modpack.toml` (options, change requests) and `REQUESTS.md` (the change requests
//!   as a to-do list for the agent).
//! - `publish <id>` runs the mod's scenarios (wasm), packages it (with its source), shows what would
//!   be uploaded, and uploads only with `--yes`: agents ask the user first.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::manifest::{ModManifest, ModPack, PackEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------- config

#[derive(Serialize, Deserialize, Default)]
struct Config {
    site: Option<String>,
    token: Option<String>,
}

fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("OPENLINA_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(base.join("openlina").join("lina.toml"))
}

fn load_config() -> Result<Config> {
    let p = config_path()?;
    if !p.exists() {
        return Ok(Config::default());
    }
    toml::from_str(&std::fs::read_to_string(&p)?).with_context(|| format!("parsing {}", p.display()))
}

fn save_config(c: &Config) -> Result<()> {
    let p = config_path()?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, toml::to_string(c)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn site_and_token() -> Result<(String, String)> {
    let c = load_config()?;
    match (c.site, std::env::var("OPENLINA_TOKEN").ok().or(c.token)) {
        (Some(s), Some(t)) => Ok((s, t)),
        _ => bail!("not logged in: run `lina login <site> <token>` (the token comes from the site's maintainer)"),
    }
}

/// The site `lina login` saved, if any.
pub fn logged_in() -> Option<String> {
    let c = load_config().ok()?;
    c.token.as_ref()?;
    c.site
}

// ---------------------------------------------------------------------- http

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

/// The body of a response, or an error with the site's `{"error": …}` message.
fn read(mut res: ureq::http::Response<ureq::Body>, what: &str) -> Result<Vec<u8>> {
    let status = res.status();
    let body = res.body_mut().with_config().limit(256 << 20).read_to_vec().with_context(|| format!("{what}: reading the response"))?;
    if !status.is_success() {
        let msg = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(String::from))
            .unwrap_or_else(|| String::from_utf8_lossy(&body).chars().take(300).collect());
        bail!("{what}: {status}: {msg}");
    }
    Ok(body)
}

fn get(url: &str, token: Option<&str>) -> Result<Vec<u8>> {
    let mut req = agent().get(url);
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    read(req.call().with_context(|| format!("GET {url}"))?, &format!("GET {url}"))
}

fn get_json(url: &str, token: Option<&str>) -> Result<Value> {
    serde_json::from_slice(&get(url, token)?).with_context(|| format!("GET {url}: not JSON"))
}

// ---------------------------------------------------------------------- login

pub fn login(site: &str, token: Option<String>) -> Result<()> {
    let site = site.trim_end_matches('/').to_string();
    ensure!(site.starts_with("http://") || site.starts_with("https://"), "the site must be an http(s) URL");
    let token = match token.or_else(|| std::env::var("OPENLINA_TOKEN").ok()) {
        Some(t) => t,
        None => {
            eprintln!("token (olt_…):");
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            line
        }
    };
    let token = token.trim().to_string();
    let me = get_json(&format!("{site}/api/me"), Some(&token)).context("checking the token")?;
    save_config(&Config { site: Some(site.clone()), token: Some(token) })?;
    println!(
        "logged in to {site} as {}{} (saved in {})",
        me["name"].as_str().unwrap_or("?"),
        if me["admin"].as_bool() == Some(true) { " (admin)" } else { "" },
        config_path()?.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------- pull

#[derive(Deserialize)]
struct Pack {
    id: String,
    url: String,
    #[serde(default)]
    zip_url: String,
    #[serde(default)]
    game_build: String,
    mods: Vec<PackMod>,
    #[serde(default)]
    section_requests: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct PackMod {
    id: String,
    name: String,
    section: String,
    version: String,
    status: String,
    package: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    options: toml::Table,
    request: Option<String>,
    #[serde(default)]
    required_by: Vec<String>,
}

/// A pack link or id → the pack's JSON URL.
fn pack_url(pack: &str) -> Result<String> {
    if pack.starts_with("http://") || pack.starts_with("https://") {
        let u = pack.trim_end_matches('/').trim_end_matches("/zip");
        ensure!(u.contains("/api/packs/"), "`{pack}` is not a pack link (…/api/packs/<id>)");
        return Ok(u.to_string());
    }
    ensure!(!pack.is_empty() && pack.chars().all(|c| c.is_ascii_alphanumeric()), "`{pack}` is neither a pack link nor a pack id");
    let site = load_config()?.site.context("a bare pack id needs `lina login <site> …` first (or pass the full link)")?;
    Ok(format!("{site}/api/packs/{pack}"))
}

pub fn pull(pack: &str, mods_dir: &Path, force: bool) -> Result<()> {
    let url = pack_url(pack)?;
    let raw = get(&url, None)?;
    let pack: Pack = serde_json::from_slice(&raw).with_context(|| format!("{url}: not a pack"))?;
    let work = PathBuf::from("work/pull").join(&pack.id);
    std::fs::create_dir_all(work.join("packages"))?;
    std::fs::write(work.join("pack.json"), &raw)?;
    println!("pack {} ({} mods, game build {})", pack.id, pack.mods.len(), pack.game_build);

    let mut notes = Vec::new();
    for m in &pack.mods {
        let bytes = get(&m.package, None)?;
        if let Some(want) = &m.sha256 {
            let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            ensure!(&got == want, "{} {}: checksum mismatch (download corrupted?)", m.id, m.version);
        }
        let zip_path = work.join("packages").join(format!("{}-{}.zip", m.id, m.version));
        std::fs::write(&zip_path, &bytes)?;
        let tmp = work.join("tmp");
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp)?;
        }
        openlina::package::unzip(Cursor::new(&bytes), &tmp)?;
        let root = if tmp.join(&m.id).join("mod.toml").is_file() { tmp.join(&m.id) } else { tmp.clone() };
        let dst = mods_dir.join(&m.id);
        let note = if dst.join("mod.toml").is_file() && !force {
            let local = ModManifest::load(&dst.join("mod.toml"))?;
            if local.info.version == m.version {
                format!("{}: have {} locally", m.id, m.version)
            } else {
                format!("{}: local {} kept, pack has {} (use --force to replace)", m.id, local.info.version, m.version)
            }
        } else if root.join("source").is_dir() {
            match check_source(&root.join("source"), &m.id) {
                Err(why) => {
                    let q = work.join("quarantine").join(&m.id);
                    if q.exists() {
                        std::fs::remove_dir_all(&q)?;
                    }
                    openlina::copy_dir(&root, &q)?;
                    format!("{}: NOT extracted, its source breaks the kit's rules ({why}); it is in {} for you to read", m.id, q.display())
                }
                Ok(()) => {
                    if dst.exists() {
                        std::fs::remove_dir_all(&dst)?;
                    }
                    std::fs::create_dir_all(&dst)?;
                    openlina::copy_dir(&root.join("source"), &dst)?;
                    std::fs::copy(root.join("mod.toml"), dst.join("mod.toml"))?;
                    for sub in ["assets", "media"] {
                        if root.join(sub).is_dir() {
                            openlina::copy_dir(&root.join(sub), &dst.join(sub))?;
                        }
                    }
                    let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                    std::fs::write(
                        dst.join(crate::build::PULLED_MARKER),
                        format!(
                            "# Pulled by `lina pull`: someone else's code. lina builds and runs it only as wasm and refuses it\n\
                             # if it reaches outside the game. Delete this file only if the user decided to trust the mod.\n\
                             pack = {:?}\nversion = {:?}\nstatus = {:?}\nsha256 = {:?}\n",
                            pack.url, m.version, m.status, sha
                        ),
                    )?;
                    format!("{}: source {} extracted to {} (pulled: runs as wasm only)", m.id, m.version, dst.display())
                }
            }
        } else {
            format!("{}: {} has no source (wasm only); it can be installed but not changed", m.id, m.version)
        };
        std::fs::remove_dir_all(&tmp)?;
        let note = if m.status == "reviewed" { note } else { format!("{note}. WARNING: {} on the site", m.status.to_uppercase()) };
        println!("  {note}");
        notes.push(note);
    }

    let modpack = ModPack {
        openlina: 1,
        game_build: Some(pack.game_build.clone()).filter(|b| !b.is_empty()),
        mods: pack
            .mods
            .iter()
            .map(|m| PackEntry {
                id: m.id.clone(),
                version: Some(m.version.clone()),
                options: m.options.clone(),
                request: m.request.clone(),
                url: Some(m.package.clone()),
                status: Some(m.status.clone()),
            })
            .collect(),
        section_requests: pack.section_requests.clone(),
    };
    modpack.save(&work.join("modpack.toml"))?;
    std::fs::write(work.join("REQUESTS.md"), requests_md(&pack, mods_dir, &notes)?)?;

    let n = pack.mods.iter().filter(|m| m.request.is_some()).count() + pack.section_requests.len();
    println!("\nwrote {}/{{pack.json, modpack.toml, REQUESTS.md}}", work.display());
    if n == 0 {
        println!("no change requests: build it with `lina build --pack {}/modpack.toml`", work.display());
    } else {
        println!("{n} change request(s): work through {}/REQUESTS.md", work.display());
    }
    Ok(())
}

fn requests_md(pack: &Pack, mods_dir: &Path, notes: &[String]) -> Result<String> {
    let dir = format!("work/pull/{}", pack.id);
    let mut s = format!(
        "# Pack {id}\n\n{url}\n\nPlayers' zip: {zip}\n\n## Mods\n\n{notes}\n",
        id = pack.id,
        url = pack.url,
        zip = pack.zip_url,
        notes = notes.iter().map(|n| format!("- {n}\n")).collect::<String>()
    );
    s.push_str(
        "## How to handle a change request\n\n\
         1. Read the mod's design notes (module doc comment of `mods/<id>/src/main.rs`) and its options in `mod.toml`.\n\
         2. If an option covers the request, set it for this pack in `{dir}/modpack.toml` (`options = { … }`) and\n   \
            you are done with that request. Otherwise change the code (see AGENTS.md) and bump the version in\n   \
            `mod.toml` (a new version on the site; the author's mod keeps its id).\n\
         3. Make the change testable: add or adjust a scenario in `mods/<id>/tests/`, run\n   \
            `lina test --mod <id>` and `lina test --mod <id> --wasm`; regenerate the showcase gif if it changed.\n\
         4. Try the whole pack: `lina build --pack {dir}/modpack.toml`, `lina run`.\n\
         5. For the player: `lina pack --from {dir}/modpack.toml --bundle pack-{id}` and
   `openlina install dist/pack-{id}.zip` (keeps the pack's options).\n\
         6. Publishing is optional and needs the user's OK in chat: `lina publish <id>` (dry run), then `--yes`.\n   \
            Only the mod's owner (or an admin) can publish new versions of an existing id.\n\n\
         ## Requests\n\n"
            .replace("{dir}", &dir)
            .replace("{id}", &pack.id)
            .as_str(),
    );
    let mut any = false;
    for m in &pack.mods {
        let Some(req) = &m.request else { continue };
        any = true;
        s.push_str(&format!("### {} ({} {}, {}, {})\n\n", m.name, m.id, m.version, m.section, m.status));
        s.push_str(&quote(req));
        s.push_str(&options_list(&mods_dir.join(&m.id), &m.options)?);
        s.push_str("- [ ] done\n\n");
    }
    for (section, req) in &pack.section_requests {
        any = true;
        let ids: Vec<&str> = pack.mods.iter().filter(|m| &m.section == section).map(|m| m.id.as_str()).collect();
        s.push_str(&format!("### All {section} in the pack ({})\n\n", ids.join(", ")));
        s.push_str(&quote(req));
        s.push_str("- [ ] done\n\n");
    }
    if !any {
        s.push_str("None.\n");
    }
    let required: Vec<String> = pack
        .mods
        .iter()
        .filter(|m| !m.required_by.is_empty())
        .map(|m| format!("{} (for {})", m.id, m.required_by.join(", ")))
        .collect();
    if !required.is_empty() {
        s.push_str(&format!("\nAdded as requirements: {}.\n", required.join("; ")));
    }
    Ok(s)
}

/// A pulled mod's source must be a plain mod crate: building it must not run code on this
/// machine (no build scripts, no dependencies beyond the kit's workspace ones, no cargo config).
fn check_source(src: &Path, id: &str) -> std::result::Result<(), String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let p = e?.path();
            out.push(p.strip_prefix(root).unwrap_or(&p).to_path_buf());
            if p.is_dir() {
                walk(&p, root, out)?;
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(src, src, &mut files).map_err(|e| e.to_string())?;
    for f in &files {
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') {
            return Err(format!("hidden file `{}`", f.display()));
        }
        if f.as_os_str() == "build.rs" || f.as_os_str() == "rust-toolchain.toml" || f.as_os_str() == "rust-toolchain" {
            return Err(format!("`{}`", f.display()));
        }
    }
    let text = std::fs::read_to_string(src.join("Cargo.toml")).map_err(|_| "no Cargo.toml".to_string())?;
    let t: toml::Table = toml::from_str(&text).map_err(|e| format!("Cargo.toml: {e}"))?;
    let workspace: toml::Table = std::fs::read_to_string("Cargo.toml")
        .ok()
        .and_then(|s| toml::from_str::<toml::Table>(&s).ok())
        .and_then(|w| w.get("workspace")?.get("dependencies")?.as_table().cloned())
        .unwrap_or_default();
    for (k, v) in &t {
        match k.as_str() {
            "package" => {
                let p = v.as_table().ok_or("[package] is not a table")?;
                for (pk, pv) in p {
                    if !["name", "version", "edition", "publish", "description", "authors", "license"].contains(&pk.as_str()) {
                        return Err(format!("[package] key `{pk}`"));
                    }
                    if pk == "name" && pv.as_str() != Some(&format!("openlina-mod-{id}")) {
                        return Err(format!("the crate must be named openlina-mod-{id}"));
                    }
                }
            }
            "bin" => {
                for b in v.as_array().ok_or("[[bin]] is not a list")? {
                    let b = b.as_table().ok_or("[[bin]] entry is not a table")?;
                    if b.keys().any(|k| k != "name" && k != "path") {
                        return Err("[[bin]] may only have name and path".into());
                    }
                    if let Some(path) = b.get("path").and_then(|p| p.as_str()) {
                        if !path.starts_with("src/") || path.contains("..") {
                            return Err(format!("[[bin]] path `{path}`"));
                        }
                    }
                }
            }
            "dependencies" => {
                for (dep, spec) in v.as_table().ok_or("[dependencies] is not a table")? {
                    let ok = spec.as_table().is_some_and(|s| s.len() == 1 && s.get("workspace").and_then(|w| w.as_bool()) == Some(true));
                    if !ok || !workspace.contains_key(dep) {
                        return Err(format!("dependency `{dep}` (only `<name>.workspace = true` of the kit's workspace dependencies)"));
                    }
                }
            }
            other => return Err(format!("Cargo.toml section `{other}`")),
        }
    }
    Ok(())
}

fn quote(text: &str) -> String {
    text.lines().map(|l| format!("> {l}\n")).collect::<String>() + "\n"
}

fn options_list(dir: &Path, chosen: &toml::Table) -> Result<String> {
    let path = dir.join("mod.toml");
    if !path.is_file() {
        return Ok(String::new());
    }
    let m = ModManifest::load(&path)?;
    if m.options.is_empty() {
        return Ok("Options: none.\n\n".into());
    }
    let mut s = String::from("Options (default → this pack):\n\n");
    for (k, o) in &m.options {
        let set = chosen.get(k).map(|v| format!(" → **{v}**")).unwrap_or_default();
        s.push_str(&format!("- `{k}` = {}{set}: {}\n", o.default, o.description));
    }
    s.push('\n');
    Ok(s)
}

// ---------------------------------------------------------------------- publish

pub fn publish(game: impl FnOnce() -> Result<PathBuf>, id: &str, yes: bool, test: bool) -> Result<()> {
    let (site, token) = site_and_token()?;
    let dir = PathBuf::from("mods").join(id);
    let m = ModManifest::load(&dir.join("mod.toml"))?;
    let version = m.info.version.clone();
    let me = get_json(&format!("{site}/api/me"), Some(&token)).context("checking the login")?;

    // Is this version new on the site?
    let res = agent().get(&format!("{site}/api/mods/{id}")).call().with_context(|| format!("GET {site}/api/mods/{id}"))?;
    let existing = if res.status() == 404 { None } else { Some(serde_json::from_slice::<Value>(&read(res, "mod lookup")?)?) };
    if let Some(e) = &existing {
        let versions: Vec<&str> = e["versions"].as_array().into_iter().flatten().filter_map(|v| v["version"].as_str()).collect();
        ensure!(!versions.contains(&version.as_str()), "{id} {version} is on the site already; bump `version` in mods/{id}/mod.toml");
        if e["uploaded_by"].as_str() != me["name"].as_str() && me["admin"].as_bool() != Some(true) {
            println!("note: {id} was uploaded by {}; the site only accepts new versions from its owner", e["uploaded_by"]);
        }
    }

    if test {
        let files = crate::scenario::find(&[], Some(id))?;
        if files.is_empty() {
            println!("warning: {id} has no scenarios (mods/{id}/tests/*.toml); publishing untested");
        } else {
            crate::scenario::test(&game()?, &files, true).context("the mod's scenarios fail (wasm); fix them before publishing")?;
        }
    }

    let out = PathBuf::from("dist");
    crate::build::pack(&[id.to_string()], None, None, &out)?;
    let zip = out.join(format!("{id}-{version}.zip"));
    let bytes = std::fs::read(&zip)?;
    let mut archive = openlina::zip::ZipArchive::new(Cursor::new(&bytes))?;
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let f = archive.by_index(i)?;
        files.push(format!("{} ({} B)", f.name(), f.size()));
    }
    let mut toml_text = String::new();
    archive.by_name(&format!("{id}/mod.toml"))?.read_to_string(&mut toml_text)?;

    println!("\nPublish to {site} as {}:", me["name"].as_str().unwrap_or("?"));
    println!("  {} {version} ({}), {:.0} KB, {}", m.info.name, id, bytes.len() as f64 / 1024.0, if existing.is_some() { "new version" } else { "new mod" });
    println!("  files: {}", files.len());
    for f in files.iter().filter(|f| !f.contains("/source/")) {
        println!("    {f}");
    }
    println!("    … and {} source files", files.iter().filter(|f| f.contains("/source/")).count());
    if !yes {
        println!("\nDry run: nothing uploaded. Ask the user; after they agree, run `lina publish {id} --yes`.");
        return Ok(());
    }
    let res = agent()
        .post(&format!("{site}/api/mods"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/zip")
        .send(&bytes[..])
        .context("uploading")?;
    let v: Value = serde_json::from_slice(&read(res, "upload")?)?;
    println!("uploaded {} {} ({}): {}", v["id"].as_str().unwrap_or(id), v["version"].as_str().unwrap_or(&version), v["status"].as_str().unwrap_or("?"), v["url"].as_str().unwrap_or(""));
    if v["status"] == "unreviewed" {
        println!("It shows as UNREVIEWED until a maintainer checks it.");
    }
    Ok(())
}
