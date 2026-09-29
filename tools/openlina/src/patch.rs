//! Running a mod's patch: bytecode in on stdin, patched bytecode out on stdout.
//!
//! Installed mods ship `patch.wasm` (a `wasm32-wasip1` command) and run in wasmtime with no
//! filesystem, network or environment access beyond their options. During development `lina`
//! runs the same code natively for speed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

#[derive(Debug, Clone)]
pub enum Patch {
    /// A `wasm32-wasip1` module, run sandboxed.
    Wasm(PathBuf),
    /// A native executable (development only).
    Native(PathBuf),
}

pub fn run(patch: &Patch, input: &[u8], id: &str, options: &str) -> Result<Vec<u8>> {
    match patch {
        Patch::Wasm(p) => run_wasm(p, input, id, options),
        Patch::Native(p) => run_native(p, input, id, options),
    }
}

fn run_native(exe: &Path, input: &[u8], id: &str, options: &str) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut child = Command::new(exe)
        .env("OPENLINA_MOD", id)
        .env("OPENLINA_OPTIONS", options)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("starting {}", exe.display()))?;
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output()?;
    writer.join().map_err(|_| anyhow!("stdin writer panicked"))??;
    if !out.status.success() {
        bail!("mod `{id}` failed ({})", out.status);
    }
    Ok(out.stdout)
}

fn run_wasm(module_path: &Path, input: &[u8], id: &str, options: &str) -> Result<Vec<u8>> {
    use wasmtime::{Config, Engine, Linker, Module, Store};
    use wasmtime_wasi::p1::{self, WasiP1Ctx};
    use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
    use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

    let engine = Engine::new(&Config::new()).map_err(|e| anyhow!("{e:#}"))?;
    let module = Module::from_file(&engine, module_path)
        .map_err(|e| anyhow!("{e:#}"))
        .with_context(|| format!("loading {}", module_path.display()))?;
    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    p1::add_to_linker_sync(&mut linker, |t| t).map_err(|e| anyhow!("{e:#}"))?;

    let stdout = MemoryOutputPipe::new(256 << 20);
    let stderr = MemoryOutputPipe::new(1 << 20);
    let wasi = WasiCtxBuilder::new()
        .stdin(MemoryInputPipe::new(input.to_vec()))
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .env("OPENLINA_MOD", id)
        .env("OPENLINA_OPTIONS", options)
        .args(&[id])
        .build_p1();
    let mut store = Store::new(&engine, wasi);
    let instance = linker.instantiate(&mut store, &module).map_err(|e| anyhow!("{e:#}"))?;
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start").map_err(|e| anyhow!("{e:#}"))?;
    let result = start.call(&mut store, ());
    let err_text = String::from_utf8_lossy(&stderr.contents()).trim_end().to_string();
    if !err_text.is_empty() {
        eprintln!("{err_text}");
    }
    match result {
        Ok(()) => {}
        Err(e) => match e.downcast_ref::<I32Exit>() {
            Some(I32Exit(0)) => {}
            Some(I32Exit(code)) => bail!("mod `{id}` failed (exit code {code})"),
            None => bail!("mod `{id}` crashed: {e:#}"),
        },
    }
    Ok(stdout.contents().to_vec())
}
