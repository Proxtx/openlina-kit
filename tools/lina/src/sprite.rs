//! `lina sprite`: pixel art from text. One character per pixel, one palette color per character.
//!
//! ```toml
//! # mods/portal-gun/art/icon.toml
//! [palette]              # optional: add or override colors (defaults: the game palette below)
//! q = "#ff00ff"
//!
//! [[frame]]              # one or more frames, same size, laid out left to right in the sheet
//! pixels = """
//! .....oo.....
//! ....o..o....
//! """
//! ```
//!
//! `lina sprite art/icon.toml --out assets/images/icon.png` writes the sheet and prints each
//! frame's rectangle (for `FrameData`); `--scale 8` writes an enlarged preview to look at.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

/// Colors sampled from the game (tiles, fruits, HUD, Lina). `.` and space are transparent.
pub const GAME_PALETTE: &[(char, &str, &str)] = &[
    ('k', "#0d0d0d", "outline / black"),
    ('w', "#ffffff", "white (text, Lina)"),
    ('g', "#a8a8a8", "light grey (UI text)"),
    ('G', "#424242", "dark grey"),
    ('f', "#212121", "play field background"),
    ('d', "#5e6175", "slate tile"),
    ('s', "#4c5b6d", "blue-grey tile"),
    ('m', "#79617b", "mauve tile"),
    ('p', "#605675", "purple tile"),
    ('P', "#5b4671", "deep purple tile"),
    ('b', "#419ecd", "fruit blue"),
    ('c', "#9fdcf5", "light blue"),
    ('o', "#e8883a", "orange"),
    ('y', "#e2c35a", "HUD yellow"),
    ('r', "#c9524a", "red"),
    ('n', "#8a6a3a", "brown (box)"),
    ('N', "#b58a4a", "light brown"),
    ('e', "#6fb35a", "leaf green"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpriteFile {
    #[serde(default)]
    palette: BTreeMap<String, String>,
    frame: Vec<Frame>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    pixels: String,
}

fn parse_color(s: &str) -> Result<[u8; 4]> {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).with_context(|| format!("bad color {s}"))?;
    Ok(match h.len() {
        6 => [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255],
        8 => [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8],
        _ => bail!("color {s}: expected #rrggbb or #rrggbbaa"),
    })
}

pub fn render(file: &Path, out: &Path, scale: u32) -> Result<()> {
    ensure!(scale >= 1, "scale must be at least 1");
    let src = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let spec: SpriteFile = toml::from_str(&src).with_context(|| format!("parsing {}", file.display()))?;
    let mut pal: BTreeMap<char, [u8; 4]> = BTreeMap::new();
    for (c, hex, _) in GAME_PALETTE {
        pal.insert(*c, parse_color(hex)?);
    }
    for (k, v) in &spec.palette {
        let mut cs = k.chars();
        let (Some(c), None) = (cs.next(), cs.next()) else { bail!("palette key `{k}` must be one character") };
        pal.insert(c, parse_color(v)?);
    }
    pal.insert('.', [0, 0, 0, 0]);
    pal.insert(' ', [0, 0, 0, 0]);

    let frames: Vec<Vec<Vec<char>>> = spec
        .frame
        .iter()
        .map(|f| f.pixels.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim_end().chars().collect()).collect())
        .collect();
    ensure!(!frames.is_empty(), "no frames");
    let h = frames[0].len();
    let w = frames[0].iter().map(|r| r.len()).max().unwrap_or(0);
    for (i, f) in frames.iter().enumerate() {
        ensure!(f.len() == h, "frame {i} has {} rows, frame 0 has {h}", f.len());
        for (y, row) in f.iter().enumerate() {
            for (x, c) in row.iter().enumerate() {
                ensure!(pal.contains_key(c), "frame {i} row {y} col {x}: `{c}` is not in the palette");
            }
        }
    }

    let (sw, sh) = (w * frames.len(), h);
    let (ow, oh) = (sw as u32 * scale, sh as u32 * scale);
    let mut buf = vec![0u8; (ow * oh * 4) as usize];
    for (i, f) in frames.iter().enumerate() {
        for (y, row) in f.iter().enumerate() {
            for (x, c) in row.iter().enumerate() {
                let rgba = pal[c];
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = ((i * w + x) as u32) * scale + dx;
                        let py = (y as u32) * scale + dy;
                        let o = ((py * ow + px) * 4) as usize;
                        buf[o..o + 4].copy_from_slice(&rgba);
                    }
                }
            }
        }
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let file_out = std::fs::File::create(out).with_context(|| format!("writing {}", out.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file_out), ow, oh);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&buf)?;
    println!("wrote {} ({ow}x{oh}, {} frame(s) of {w}x{h})", out.display(), frames.len());
    if scale == 1 {
        for i in 0..frames.len() {
            println!("  frame {i}: x={} y=0 w={w} h={h}", i * w);
        }
    }
    Ok(())
}

pub fn print_palette() {
    for (c, hex, what) in GAME_PALETTE {
        println!("{c}  {hex}  {what}");
    }
    println!(".  transparent");
}
