//! Sounds: play the game's sound effects, or a mod's own, the way the game does.
//!
//! The game plays effects with `EvSheet.sfx(name, loop, volume, pitch, object)` (fn@3432): `name` is a
//! file of `res/media/` without its extension (`gungun_shot`, `portal_open_win`, …; ~150 of them,
//! listed in docs/game/engine.md "Sounds"), `volume` an offset in dB (0 = as recorded, the game often uses -3..-10),
//! `pitch` a random pitch spread (the game uses 0.2 for its gun: repeated shots don't sound the same), `loop`
//! 0. A name already played this frame is skipped (`sfxThisFrame`). The sound player (`PAudio`) lists
//! `res/media/` once when the game starts, so a mod's own sound is a file in its `assets/media/` (WAV or
//! OGG, a name of its own like `mymod_zap.wav`; no subfolders) and is then played by that name. A name
//! the sound player doesn't know would crash the game inside `sfx` (null access); `play` checks
//! `PAudio.files` first and skips it.
//!
//! ```ignore
//! // in a handler that has the gameplay sheet (item_use: f.arg(1); tick: f.arg(0))
//! sound::play(&mut f, sheet, &Sound::new("gungun_shot").volume(-3.0))?;
//! ```

use anyhow::Result;
use hlbc::opcodes::Opcode;
use hlbc::types::{Reg, Type};

use crate::asm::FnBuilder;

/// The class whose `sfx` plays sounds (the gameplay sheet and the others extend it).
const SHEET: &str = "fish.game.evsheet.EvSheet";

/// A sound to play: a name in `res/media/` (without extension) and how.
#[derive(Clone, Debug)]
pub struct Sound<'a> {
    pub name: &'a str,
    /// Offset in dB (0: as recorded; negative is quieter).
    pub volume: f64,
    /// Random pitch spread per play (0: always the same; the game uses about 0.1-0.2).
    pub pitch: f64,
}

impl<'a> Sound<'a> {
    pub fn new(name: &'a str) -> Self {
        Self { name, volume: 0.0, pitch: 0.1 }
    }
    pub fn volume(mut self, db: f64) -> Self {
        self.volume = db;
        self
    }
    pub fn pitch(mut self, spread: f64) -> Self {
        self.pitch = spread;
        self
    }
}

/// Play `sound` through `sheet` (any `EvSheet`, e.g. the gameplay sheet a handler gets).
pub fn play(f: &mut FnBuilder, sheet: Reg, sound: &Sound) -> Result<()> {
    let sfx = f.code().method(SHEET, "sfx")?;
    let sheet_t = f.code().class(SHEET)?;
    let (i32_t, f64_t) = (f.code().ty_i32(), f.code().ty_f64());
    let (ref_i32, ref_f64) = (f.code().intern_type(Type::Ref(i32_t)), f.code().intern_type(Type::Ref(f64_t)));
    let object_t = f.code().func_type(sfx)?.args[5];

    let first = f.code().method("fish.system.Picker", "first")?;
    let exists = f.code().method("haxe.ds.StringMap", "exists")?;
    let audio_t = f.code().class("fish.game.oclass.OClass_Audio")?;

    // A name the sound player doesn't have would end in a null access inside `sfx`: skip it
    // (`Audio.first().pAudio.files.exists(name)`).
    let skip = f.label();
    let this = f.cast(sheet, sheet_t);
    let name = f.string_obj(sound.name)?;
    let audios = f.get_new(this, "Audio")?;
    f.jnull(audios, skip);
    let a = f.call_new(first, &[audios])?;
    let audio = f.cast(a, audio_t);
    f.jnull(audio, skip);
    let player = f.get_new(audio, "pAudio")?;
    f.jnull(player, skip);
    let files = f.get_new(player, "files")?;
    f.jnull(files, skip);
    let known = f.call_new(exists, &[files, name])?;
    f.jfalse(known, skip);
    let (looping, volume, pitch) = (f.reg_i32(), f.reg_f64(), f.reg_f64());
    f.int(looping, 0);
    f.float(volume, sound.volume);
    f.float(pitch, sound.pitch);
    let (looping_ref, volume_ref, pitch_ref) = (f.reg(ref_i32), f.reg(ref_f64), f.reg(ref_f64));
    f.op(Opcode::Ref { dst: looping_ref, src: looping });
    f.op(Opcode::Ref { dst: volume_ref, src: volume });
    f.op(Opcode::Ref { dst: pitch_ref, src: pitch });
    let object = f.reg(object_t);
    f.op(Opcode::Null { dst: object });
    f.call_new(sfx, &[this, name, looping_ref, volume_ref, pitch_ref, object])?;
    f.place(skip);
    Ok(())
}
