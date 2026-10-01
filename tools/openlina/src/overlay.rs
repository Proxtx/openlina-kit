//! The overlay game directory.
//!
//! The game finds its resources (`./fish/game/res`) and saves (`./userdata`) relative to its
//! working directory. The overlay is a directory of symlinks to every entry of the install, with
//! the patched `hlboot.dat` on top. When mods ship assets, `fish/` is mirrored with hard links
//! (symlinks inside the resource tree confuse Heaps' path handling) and the assets are copied in.
//! Running the game there leaves the install untouched, and `userdata` stays shared with the
//! vanilla game. PNGs the game can't load (`openlina_sdk::assets::png_problem`) are copied in
//! as 8-bit RGBA.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use openlina_sdk::assets::{is_png, png_problem};

/// (Re)create `overlay` for `game` with `bytecode` and `assets` (paths relative to
/// `fish/game/res`, source files). Returns a note per PNG it had to convert.
pub fn create(game: &Path, overlay: &Path, bytecode: &[u8], assets: &[(PathBuf, PathBuf)]) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    if overlay.exists() {
        // Only symlinks and files we created live here; symlinks are removed, not followed.
        std::fs::remove_dir_all(overlay).with_context(|| format!("removing {}", overlay.display()))?;
    }
    std::fs::create_dir_all(overlay)?;
    for e in std::fs::read_dir(game)? {
        let e = e?;
        if e.file_name() == "hlboot.dat" {
            continue;
        }
        symlink(&e.path(), &overlay.join(e.file_name()))?;
    }
    std::fs::write(overlay.join("hlboot.dat"), bytecode)?;

    if !assets.is_empty() {
        // Heaps resolves resource paths through realpath and slices them against its base
        // directory, so symlinked entries below the resource root break it. Mirror `fish/` as
        // real directories with hard links (no disk space; copies across filesystems).
        let fish = overlay.join("fish");
        std::fs::remove_file(&fish)?;
        mirror(&game.join("fish"), &fish)?;
    }
    let res = Path::new("fish/game/res");
    for (rel, src) in assets {
        let dst = overlay.join(res).join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dst.symlink_metadata().is_ok() {
            std::fs::remove_file(&dst)?; // a mod asset replaces a game file of the same name
        }
        if is_png(src) {
            let bytes = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
            if let Some(why) = png_problem(&bytes) {
                let fixed = to_rgba8(&bytes).with_context(|| format!("{}: {why}", src.display()))?;
                std::fs::write(&dst, fixed)?;
                notes.push(format!("converted {} to 8-bit RGBA: {why}", src.display()));
                continue;
            }
        }
        std::fs::copy(src, &dst).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(notes)
}

/// Re-encode a PNG as 8-bit RGBA, which every game build reads.
pub fn to_rgba8(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut dec = png::Decoder::new(bytes);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("the palette was not expanded"),
    };
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, info.width, info.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    w.write_image_data(&rgba)?;
    w.finish()?;
    Ok(out)
}

/// Recreate `src` at `dst`: real directories, files hard-linked (copied if that fails).
fn mirror(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let (from, to) = (e.path(), dst.join(e.file_name()));
        if e.file_type()?.is_dir() {
            mirror(&from, &to)?;
        } else if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dst).with_context(|| format!("linking {}", dst.display()))
}

#[cfg(windows)]
fn symlink(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dst)
    } else {
        std::os::windows::fs::symlink_file(src, dst)
    }
    .with_context(|| format!("linking {}", dst.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_4_bit_palette_png() {
        // 2x1, 4-bit palette: red, then transparent green
        let mut src = Vec::new();
        let mut enc = png::Encoder::new(&mut src, 2, 1);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Four);
        enc.set_palette(vec![255, 0, 0, 0, 255, 0]);
        enc.set_trns(vec![255, 0]);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0x01]).unwrap();
        w.finish().unwrap();
        assert!(png_problem(&src).unwrap().starts_with("4-bit palette"));

        let out = to_rgba8(&src).unwrap();
        assert_eq!(png_problem(&out), None);
        let mut r = png::Decoder::new(&out[..]).read_info().unwrap();
        let mut px = vec![0; r.output_buffer_size()];
        r.next_frame(&mut px).unwrap();
        assert_eq!(px, [255, 0, 0, 255, 0, 255, 0, 0]);
    }
}
