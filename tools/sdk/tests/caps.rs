//! The capability check against the real game bytecode (skipped without `work/hlboot.orig.dat`).

use std::path::Path;

use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::caps::{diff, Snapshot};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::Code;

fn load() -> Option<Code> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/hlboot.orig.dat");
    if !p.exists() {
        eprintln!("skipping: {} not found (run `lina setup`)", p.display());
        return None;
    }
    Some(Code::load(p).unwrap())
}

/// Apply `patch` to a fresh copy and return the categories found.
fn check(patch: impl FnOnce(&mut Code)) -> Option<Vec<String>> {
    let before = load()?;
    let snap = Snapshot::of(&before);
    let mut after = load()?;
    patch(&mut after);
    let after = Code::from_bytes(&after.to_bytes().unwrap()).unwrap();
    Some(diff(&snap, &before, &after).iter().map(|f| f.to_string()).collect())
}

fn new_fn(code: &mut Code, body: impl FnOnce(&mut FnBuilder)) {
    let void = code.ty_void();
    let mut f = FnBuilder::new(code, "test/caps", &[], void);
    body(&mut f);
    f.ret_void();
    f.finish().unwrap();
}

#[test]
fn gameplay_code_is_clean() {
    let Some(found) = check(|code| {
        new_fn(code, |f| f.print(&[Print::Str("hello")]).unwrap());
    }) else {
        return;
    };
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn file_and_program_access_is_found() {
    let Some(found) = check(|code| {
        let get = code.method("sys.io.File", "getContent").unwrap();
        let cmd = code.method("Sys", "command").unwrap();
        let arr_t = code.func_type(cmd).unwrap().args[1];
        new_fn(code, |f| {
            let path = f.string_obj("/home/x/.ssh/id_ed25519").unwrap();
            f.call_new(get, &[path]).unwrap();
            let args = f.reg(arr_t);
            f.op(Opcode::Null { dst: args });
            let prog = f.string_obj("rm").unwrap();
            f.call_new(cmd, &[prog, args]).unwrap();
        });
    }) else {
        return;
    };
    assert!(found.iter().any(|f| f.starts_with("[files]") && f.contains("getContent")), "{found:?}");
    assert!(found.iter().any(|f| f.starts_with("[programs]") && f.contains("command")), "{found:?}");
}

#[test]
fn tampering_with_existing_code_and_constants_is_found() {
    let Some(found) = check(|code| {
        // change an existing function that reads files, and a string constant
        let get = code.method("sys.io.File", "getContent").unwrap();
        let f = code.func_mut(get).unwrap();
        openlina_sdk::edit::insert_ops(f, 0, vec![Opcode::Label], openlina_sdk::edit::Incoming::ToOriginal);
        let i = code.bc.strings.iter().position(|s| s.as_str() == "userdata").unwrap();
        code.bc.strings[i] = "../../.ssh".into();
    }) else {
        return;
    };
    assert!(
        found.iter().any(|f| f.starts_with("[files]") && f.contains("changes `sys.io.$File.getContent`")),
        "{found:?}"
    );
    assert!(found.iter().any(|f| f.starts_with("[constants]") && f.contains("userdata")), "{found:?}");
}
