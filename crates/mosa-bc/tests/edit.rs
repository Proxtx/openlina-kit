//! Tests against the real game bytecode. They are skipped when `work/hlboot.orig.dat` is missing
//! (run `mosa setup` first); the game files are not part of the repository.

use std::path::Path;

use mosa_bc::edit::{insert_ops, jump_targets, remove_ops, Incoming};
use mosa_bc::hlbc::opcodes::Opcode;
use mosa_bc::{validate, Code};

fn load() -> Option<Code> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/hlboot.orig.dat");
    if !p.exists() {
        eprintln!("skipping: {} not found (run `mosa setup`)", p.display());
        return None;
    }
    Some(Code::load(p).unwrap())
}

/// Inserting ops must keep every original jump pointing at the same original instruction,
/// and removing them again must give back the exact original function.
#[test]
fn insert_then_remove_is_identity_everywhere() {
    let Some(code) = load() else { return };
    let mut checked = 0;
    for fun in code.bc.functions.iter().filter(|f| f.ops.len() > 8).step_by(7) {
        for at in [0, fun.ops.len() / 3, fun.ops.len() / 2, fun.ops.len() - 1] {
            let mut f = fun.clone();
            insert_ops(&mut f, at, vec![Opcode::Nop, Opcode::Nop], Incoming::ToOriginal);
            let map = |i: usize| if i < at { i } else { i + 2 };
            for (p, op) in fun.ops.iter().enumerate() {
                let before: Vec<_> = jump_targets(op, p).into_iter().map(map).collect();
                assert_eq!(jump_targets(&f.ops[map(p)], map(p)), before, "fn@{} op {p} at {at}", fun.findex.0);
            }
            assert!(validate::check_function(&code, &f).is_empty());
            remove_ops(&mut f, at..at + 2).unwrap();
            assert_eq!(format!("{:?}", f.ops), format!("{:?}", fun.ops));
            assert_eq!(f.debug_info, fun.debug_info);
            checked += 1;
        }
    }
    assert!(checked > 1000);
}

/// `Incoming::ToInserted` redirects forward jumps that targeted `at` to the inserted code.
#[test]
fn to_inserted_redirects_forward_jumps() {
    let Some(code) = load() else { return };
    let fun = code.bc.functions.iter().find(|f| {
        f.ops.iter().enumerate().any(|(p, op)| jump_targets(op, p).iter().any(|&t| t > p + 1))
    });
    let fun = fun.unwrap();
    let (p, t) = fun
        .ops
        .iter()
        .enumerate()
        .find_map(|(p, op)| jump_targets(op, p).into_iter().find(|&t| t > p + 1).map(|t| (p, t)))
        .unwrap();
    let mut f = fun.clone();
    insert_ops(&mut f, t, vec![Opcode::Nop], Incoming::ToInserted);
    assert!(jump_targets(&f.ops[p], p).contains(&t));
    assert!(matches!(f.ops[t], Opcode::Nop));
}

#[test]
fn vanilla_code_validates_cleanly() {
    let Some(code) = load() else { return };
    let problems: usize = code.bc.functions.iter().map(|f| validate::check_function(&code, f).len()).sum();
    assert_eq!(problems, 0);
}
