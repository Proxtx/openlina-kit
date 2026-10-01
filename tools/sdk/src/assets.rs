//! Files mods ship in `assets/` (laid over the game's `fish/game/res/`) and whether the game can
//! load them.
//!
//! The game reads PNG headers in `hxd.res.Image.getInfo`: 8-bit PNGs of every color type load,
//! 16-bit ones without a palette too; any other bit depth (1, 2 or 4: small palette or grayscale
//! images, which image editors and optimizers like to write) throws `Unsupported png format
//! <bits>/<type>`. The game loads mod images while it starts, so one such file leaves it frozen
//! on a black screen. `lina pack` and the website refuse those files; `openlina` converts them
//! when it builds the overlay, so packages made before the check still play.

use std::path::Path;

/// The command that turns `file` into a PNG every game build reads (8-bit RGBA), for messages.
pub fn png_fix(file: &str) -> String {
    format!("magick {file} PNG32:{file}")
}

/// Whether `path` is a file the game reads as a PNG (by extension, like the game).
pub fn is_png(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// Why the game can't load these PNG bytes, or `None` when it can.
pub fn png_problem(bytes: &[u8]) -> Option<String> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 26 || !bytes.starts_with(SIG) || &bytes[12..16] != b"IHDR" {
        return Some("not a PNG file".into());
    }
    let (bits, color) = (bytes[24], bytes[25]);
    let ok = match bits {
        8 => true,
        16 => color != 3,
        _ => false,
    };
    (!ok).then(|| {
        let kind = match color {
            0 => "grayscale",
            2 => "RGB",
            3 => "palette",
            4 => "grayscale with alpha",
            6 => "RGBA",
            _ => "unknown color type",
        };
        format!("{bits}-bit {kind} PNG (the game reads 8-bit PNGs, and 16-bit ones without a palette)")
    })
}

/// The PNG files under a mod's `assets/` the game can't load: (path relative to `assets/`, why).
pub fn png_problems(assets_dir: &Path) -> std::io::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    if assets_dir.is_dir() {
        walk(assets_dir, assets_dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(root, &p, out)?;
        } else if is_png(&p) {
            if let Some(why) = png_problem(&std::fs::read(&p)?) {
                let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                out.push((rel, why));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(bits: u8, color: u8) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x18\0\0\0\x18".to_vec();
        b.extend([bits, color, 0, 0, 0]);
        b
    }

    #[test]
    fn the_games_rule() {
        for color in [0, 2, 3, 4, 6] {
            assert_eq!(png_problem(&header(8, color)), None, "8-bit type {color}");
        }
        for color in [0, 2, 4, 6] {
            assert_eq!(png_problem(&header(16, color)), None, "16-bit type {color}");
        }
        assert!(png_problem(&header(16, 3)).is_some());
        for bits in [1, 2, 4] {
            assert!(png_problem(&header(bits, 3)).unwrap().contains(&format!("{bits}-bit palette")));
            assert!(png_problem(&header(bits, 0)).is_some());
        }
        assert_eq!(png_problem(b"GIF89a....").as_deref(), Some("not a PNG file"));
    }
}
