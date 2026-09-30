use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod build;
mod docs;
mod doctor;
mod dump;
mod hub;
mod inspect;
mod scaffold;
mod scenario;
mod sprite;

/// Pristine bytecode copied from the game by `lina setup`.
pub const ORIG: &str = "work/hlboot.orig.dat";
/// Output of `lina build`.
pub const MODDED: &str = "work/hlboot.modded.dat";
/// Overlay game directory used by `lina run`.
pub const OVERLAY: &str = "work/game";

#[derive(Parser)]
#[command(name = "lina", about = "OpenLina mod development CLI", version)]
struct Cli {
    /// Game install directory (default: $MOSA_GAME_DIR, then the default Steam library).
    #[arg(long, global = true)]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Copy the pristine game bytecode to work/hlboot.orig.dat and check the game version.
    Setup,
    /// Decompile the whole game into work/dump (pseudo-Haxe, exact disassembly, index files).
    Dump {
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
        #[arg(long, default_value = "work/dump")]
        out: PathBuf,
    },
    /// Show one function: disassembly, and decompiled pseudo-Haxe with --hx.
    /// FUNC is a findex (`3767`) or `Class.method` (`fish.game.evsheet.EvSheet_gameplay.update`).
    Fn {
        func: String,
        #[arg(long)]
        hx: bool,
        /// Only print ops in this range, e.g. `15300..15400`.
        #[arg(long)]
        ops: Option<String>,
        /// Bytecode file to read (e.g. work/hlboot.modded.dat to inspect a patch).
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// List every function that calls FUNC directly or makes a closure of it.
    Callers {
        func: String,
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// Search the string constant pool and show which functions use matching strings.
    Strings {
        pattern: String,
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// Every function that reads or writes a field of this name or uses this string (also the
    /// game's string globals, which `strings` misses).
    Refs {
        name: String,
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// A class's fields (with types), methods and statics; the package may be left out.
    Class {
        name: String,
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// Regenerate the option tables in docs/mods.md from the mods' mod.toml (`--check`: fail if
    /// one is out of date).
    Docs {
        #[arg(long)]
        check: bool,
    },
    /// List the mods in mods/ and their options.
    Mods,
    /// List the hooks the `core` mod provides (subscribe with `openlina_sdk::hooks`).
    Hooks,
    /// Create a new mod crate from a template: `lina new items portal-gun`.
    New { section: String, id: String },
    /// Build mods and apply them to the pristine bytecode (+ overlay in work/game).
    Build {
        /// Modpack selecting mods and options.
        #[arg(long, default_value = "modpack.toml")]
        pack: PathBuf,
        /// Build only these mods (their `requires` are added), with default options.
        #[arg(long = "mod")]
        only: Vec<String>,
        /// Run the wasm builds (what players get) instead of native ones.
        #[arg(long)]
        wasm: bool,
        #[arg(long, default_value = MODDED)]
        out: PathBuf,
    },
    /// Launch the game from the work/game overlay (build first with --build).
    Run {
        #[arg(long)]
        build: bool,
        #[arg(long, default_value = "modpack.toml")]
        pack: PathBuf,
        /// Kill the game after this many seconds (useful for automated smoke tests).
        #[arg(long)]
        timeout: Option<u64>,
        /// No window: SDL's offscreen driver (EGL). Combine with the `autostart` mod to reach a
        /// level without input.
        #[arg(long)]
        headless: bool,
        /// Record your play (mod `record`) and turn every level attempt into a scenario that
        /// replays it, in work/recordings/ (implies --build).
        #[arg(long)]
        record: bool,
    },
    /// Turn a log with `[record]` lines (from `lina run --record` or a test with the fixture
    /// `record`) into replay scenarios in work/recordings/.
    Recordings {
        log: PathBuf,
        /// Mods the scenarios load (default: the non-dev mods of modpack.toml).
        #[arg(long = "mod")]
        mods: Vec<String>,
    },
    /// Build wasm packages into dist/: one `<id>-<version>.zip` per mod, and with --bundle a
    /// pack zip (helper + modpack.toml + mods) ready for players.
    Pack {
        /// Mods to package (default: all non-dev mods).
        ids: Vec<String>,
        /// Also write dist/<NAME>.zip containing the helper and these mods.
        #[arg(long)]
        bundle: Option<String>,
        /// Take the mods and their options from a modpack (e.g. work/pull/<pack>/modpack.toml);
        /// the bundle keeps the options.
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long, default_value = "dist")]
        out: PathBuf,
    },
    /// Run test scenarios headless (default: every mods/*/tests/*.toml) and check their logs.
    Test {
        /// Scenario files (default: all).
        files: Vec<PathBuf>,
        /// Only the scenarios of this mod.
        #[arg(long = "mod")]
        only: Option<String>,
        /// Only scenarios whose name or file contains this text.
        #[arg(short = 'k', long)]
        filter: Option<String>,
        /// Only the scenarios that failed in the last run.
        #[arg(long)]
        failed: bool,
        /// Use the wasm builds (what players run).
        #[arg(long)]
        wasm: bool,
        /// Scenarios to run at the same time (default: half the cores, at most 8).
        #[arg(short = 'j', long)]
        jobs: Option<usize>,
    },
    /// Look at the running game: print parts of its state (fixture `inspect`) at chosen moments,
    /// without writing a scenario or a mod. E.g.
    /// `lina probe --mod swap --level "greendemo 1" --at tick:30 game.itemManager.itemPool.length`.
    Probe {
        /// Paths to print (see mods/inspect/mod.toml): `game.a.b[].c`, `@<type>[].field`, `$<class>.<static>`.
        paths: Vec<String>,
        /// Mods to load (repeatable).
        #[arg(long = "mod")]
        mods: Vec<String>,
        /// Level to load by name (default: the run's first level).
        #[arg(long)]
        level: Option<String>,
        /// Moments (repeatable): `tick:N` or `layout:<name>@N`.
        #[arg(long, default_value = "tick:30")]
        at: Vec<String>,
        /// Play the game's own run start (hub, tool selection) instead of loading a level.
        #[arg(long)]
        new_run: bool,
        /// Scripted inputs (repeatable), e.g. `30-700:right`.
        #[arg(long)]
        input: Vec<String>,
        /// Capture frames `from-to/step` (a contact sheet is written).
        #[arg(long)]
        capture: Option<String>,
        /// End at this tick (default: 60 after the last `tick:` moment, or 1200).
        #[arg(long)]
        end: Option<u32>,
        /// Seed.
        #[arg(long, default_value_t = 1)]
        seed: i64,
        /// Any harness option (repeatable), e.g. `--harness modifier_key=moon-gravity`.
        #[arg(long = "harness")]
        harness: Vec<String>,
        /// A mod option (repeatable), e.g. `--set moon-gravity.factor=0.25`.
        #[arg(long = "set")]
        set: Vec<String>,
    },
    /// Record a scenario's [gif] section: capture frames headless, write the gif (default: the
    /// scenario's gif.out, relative to its mod directory).
    Gif {
        scenario: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Render pixel art from a text sprite file to PNG (see `lina sprite --palette`).
    Sprite {
        file: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Enlarge every pixel (previews).
        #[arg(long, default_value_t = 1)]
        scale: u32,
        /// Print the built-in game palette.
        #[arg(long)]
        palette: bool,
    },
    /// Save an OpenLina site and your upload token (checked against the site) for pull/publish.
    Login {
        /// The site, e.g. `http://127.0.0.1:8080`.
        site: String,
        /// Upload token (`olt_…`); read from $OPENLINA_TOKEN or stdin when omitted.
        token: Option<String>,
    },
    /// Download a pack from an OpenLina site: packages, the source of mods you don't have
    /// (into mods/<id>/), and work/pull/<pack>/ with modpack.toml and REQUESTS.md (change requests).
    Pull {
        /// Pack link (`https://site/api/packs/<id>`) or pack id (on the site you logged in to).
        pack: String,
        /// Where to put the source of pulled mods.
        #[arg(long, default_value = "mods")]
        mods_dir: PathBuf,
        /// Replace local mods of the same id with the pack's version.
        #[arg(long)]
        force: bool,
    },
    /// Upload a mod to the site you logged in to: runs its scenarios (wasm), packages it with its
    /// source, shows what would be uploaded. Uploads only with --yes (ask the user first).
    Publish {
        id: String,
        /// Really upload.
        #[arg(long)]
        yes: bool,
        /// Skip the scenarios.
        #[arg(long)]
        no_test: bool,
    },
    /// Check this machine: toolchain (with or without nix), wasm target, ImageMagick, the game, setup.
    Doctor,
    /// Self-test: roundtrip the pristine bytecode and run the validator on every function.
    Check {
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let game_dir = || openlina::game::game_dir(cli.game_dir.clone());
    match cli.cmd {
        Cmd::Setup => build::setup(&game_dir()?),
        Cmd::Dump { input, out } => dump::run(&input, &out),
        Cmd::Fn { func, hx, ops, input } => inspect::show_fn(&input, &func, hx, ops.as_deref()),
        Cmd::Callers { func, input } => inspect::callers(&input, &func),
        Cmd::Strings { pattern, input } => inspect::strings(&input, &pattern),
        Cmd::Refs { name, input } => inspect::refs(&input, &name),
        Cmd::Class { name, input } => inspect::class(&input, &name),
        Cmd::Docs { check } => docs::docs(check),
        Cmd::Mods => build::list_mods(),
        Cmd::Hooks => {
            for h in openlina_sdk::hooks::CORE_HOOKS {
                println!(
                    "{}{} -> {}\n    {}\n",
                    h.name,
                    h.args,
                    h.returns,
                    h.doc.split_whitespace().collect::<Vec<_>>().join(" ")
                );
            }
            Ok(())
        }
        Cmd::New { section, id } => scaffold::new_mod(&section, &id),
        Cmd::Build { pack, only, wasm, out } => build::build(&game_dir()?, &pack, &only, wasm, &out),
        Cmd::Run { build, pack, timeout, headless, record } => {
            let game = game_dir()?;
            if record {
                return scenario::run_recording(&game, &pack, timeout, headless);
            }
            if build {
                build::build(&game, &pack, &[], false, &PathBuf::from(MODDED))?;
            }
            build::run(&game, timeout, headless)
        }
        Cmd::Recordings { log, mods } => {
            let text = std::fs::read_to_string(&log)?;
            let mods = if mods.is_empty() { scenario::pack_mods(&PathBuf::from("modpack.toml"))? } else { mods };
            for p in scenario::recordings(&text, &mods)? {
                println!("wrote {}", p.display());
            }
            Ok(())
        }
        Cmd::Pack { ids, bundle, from, out } => {
            let from = from.map(|p| openlina_sdk::manifest::ModPack::load(&p)).transpose()?;
            let mut ids = ids;
            if let Some(f) = &from {
                anyhow::ensure!(ids.is_empty(), "give either mod ids or --from, not both");
                ids = f.mods.iter().map(|e| e.id.clone()).collect();
            }
            build::pack(&ids, bundle.as_deref(), from.as_ref(), &out)
        }
        Cmd::Test { files, only, filter, failed, wasm, jobs } => {
            let mut files = if failed { scenario::last_failed()? } else { scenario::find(&files, only.as_deref())? };
            if let Some(k) = &filter {
                files.retain(|f| {
                    f.to_string_lossy().contains(k.as_str())
                        || scenario::Scenario::load(f).is_ok_and(|s| s.name.contains(k.as_str()))
                });
            }
            let opts = scenario::TestOpts { wasm, jobs: jobs.unwrap_or_else(scenario::default_jobs) };
            scenario::test(&game_dir()?, &files, &opts)
        }
        Cmd::Probe { paths, mods, level, at, new_run, input, capture, end, seed, harness, set } => scenario::probe(
            &game_dir()?,
            &scenario::ProbeSpec { paths, mods, level, at, new_run, inputs: input, capture, end, seed, harness, set },
        ),
        Cmd::Gif { scenario, out } => scenario::gif(&game_dir()?, &scenario, out.as_deref()),
        Cmd::Sprite { file, out, scale, palette } => {
            if palette {
                sprite::print_palette();
                return Ok(());
            }
            let file = file.ok_or_else(|| anyhow::anyhow!("give a sprite file (or --palette)"))?;
            let out = out.unwrap_or_else(|| file.with_extension("png"));
            sprite::render(&file, &out, scale)
        }
        Cmd::Check { input } => build::check(&input),
        Cmd::Doctor => doctor::doctor(cli.game_dir.clone()),
        Cmd::Login { site, token } => hub::login(&site, token),
        Cmd::Pull { pack, mods_dir, force } => hub::pull(&pack, &mods_dir, force),
        Cmd::Publish { id, yes, no_test } => hub::publish(game_dir, &id, yes, !no_test),
    }
}
