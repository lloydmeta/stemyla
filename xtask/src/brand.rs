//! `xtask brand`: draws the page's icons and its share image into
//! `app/assets/`.
//!
//! ```text
//! cargo run -p xtask -- brand           # writes the files
//! cargo run -p xtask -- brand --check   # fails when a file differs from what brand draws
//! ```
//!
//! The files:
//!
//! ```text
//! icon.svg               the favicon: six bars in the stem colours on a dark tile
//! favicon.ico            the same at 32 px, one PNG in an ICO, for browsers without SVG icons
//! apple-touch-icon.png   the same at 180 px on a square tile, for the iPhone home screen
//! og.png                 the share image (1200 x 630): a mix that splits into six stem lanes
//! ```
//!
//! `app/index.html` copies the files into each build, and names them in
//! its head tags.  [`head_problems`] checks those tags.  `xtask publish`
//! also puts `og.png` at the site's root, where the `og:image` URL points.
//!
//! The drawing uses only plain arithmetic and square roots, and no
//! platform maths such as `exp`, so each machine draws the same pixels.
//! The waveforms come from a seeded generator: they look like a song, but
//! no audio makes them.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use anyhow::{Context, Result, bail};
use std::path::Path;

pub const USAGE: &str = "usage: xtask brand [--check]";

/// The folder of the files, from the workspace's root.
pub const ASSETS: &str = "app/assets";

pub const ICON_SVG: &str = "icon.svg";
pub const FAVICON_ICO: &str = "favicon.ico";
pub const TOUCH_ICON: &str = "apple-touch-icon.png";
pub const OG_PNG: &str = "og.png";

/// The names of the files that [`files`] draws, in its order.
pub const FILES: [&str; 4] = [ICON_SVG, FAVICON_ICO, TOUCH_ICON, OG_PNG];

pub const FAVICON_SIZE: u32 = 32;
pub const TOUCH_ICON_SIZE: u32 = 180;
pub const OG_WIDTH: u32 = 1200;
pub const OG_HEIGHT: u32 = 630;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// The lanes' colours, in the app's order, as `stem_color` in
/// `app/src/lib.rs` gives them.
pub const STEMS: [(&str, Rgb); 6] = [
    ("bass", Rgb(90, 140, 230)),
    ("drums", Rgb(230, 90, 70)),
    ("other", Rgb(90, 200, 200)),
    ("vocals", Rgb(200, 110, 210)),
    ("guitar", Rgb(240, 170, 60)),
    ("piano", Rgb(120, 200, 120)),
];

/// The app's mix lane.
pub const MIX: Rgb = Rgb(220, 220, 220);

/// The page's background, and the icons' tile.
pub const TILE: Rgb = Rgb(0x1b, 0x1b, 0x1b);

const OG_BACKGROUND: Rgb = Rgb(18, 18, 18);
const WORDMARK: Rgb = Rgb(242, 242, 242);
const SUBTITLE: Rgb = Rgb(185, 185, 185);
const FOOTER: Rgb = Rgb(138, 138, 138);

/// An image with straight (not premultiplied) RGBA, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

/// Premultiplied RGBA from 0 to 1, row by row.
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[f64; 4]>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        let (width, height) = (width as usize, height as usize);
        Canvas {
            width,
            height,
            pixels: vec![[0.0; 4]; width * height],
        }
    }

    /// Paints `colour` over the pixel at `alpha`, which holds the coverage.
    fn blend(&mut self, x: i64, y: i64, colour: Rgb, alpha: f64) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height || alpha <= 0.0 {
            return;
        }
        let a = alpha.min(1.0);
        let src = [colour.0, colour.1, colour.2].map(|c| f64::from(c) / 255.0 * a);
        let p = &mut self.pixels[y as usize * self.width + x as usize];
        for c in 0..3 {
            p[c] = src[c] + p[c] * (1.0 - a);
        }
        p[3] = a + p[3] * (1.0 - a);
    }

    /// Fills `r` with exact coverage at its fractional edges.
    fn fill_rect(&mut self, r: Rect, colour: Rgb, alpha: f64) {
        let overlap = |p: f64, lo: f64, hi: f64| ((p + 1.0).min(hi) - p.max(lo)).max(0.0);
        for py in r.y0.floor() as i64..r.y1.ceil() as i64 {
            let cy = overlap(py as f64, r.y0, r.y1);
            for px in r.x0.floor() as i64..r.x1.ceil() as i64 {
                self.blend(px, py, colour, alpha * cy * overlap(px as f64, r.x0, r.x1));
            }
        }
    }

    /// Fills `r` with corners of `radius`, from 4 x 4 samples per pixel.
    fn fill_round_rect(&mut self, r: Rect, radius: f64, colour: Rgb) {
        const N: usize = 4;
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        let (hw, hh) = ((r.x1 - r.x0) / 2.0, (r.y1 - r.y0) / 2.0);
        let inside = |x: f64, y: f64| {
            let qx = (x - cx).abs() - (hw - radius);
            let qy = (y - cy).abs() - (hh - radius);
            let outside = (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt();
            outside + qx.max(qy).min(0.0) - radius <= 0.0
        };
        for py in r.y0.floor() as i64..r.y1.ceil() as i64 {
            for px in r.x0.floor() as i64..r.x1.ceil() as i64 {
                let hits = (0..N * N)
                    .filter(|i| {
                        let sx = px as f64 + ((i % N) as f64 + 0.5) / N as f64;
                        let sy = py as f64 + ((i / N) as f64 + 0.5) / N as f64;
                        inside(sx, sy)
                    })
                    .count();
                self.blend(px, py, colour, hits as f64 / (N * N) as f64);
            }
        }
    }

    /// Writes `text` from `x` on the `baseline`, at a CSS font size of
    /// `size` px.
    fn text(&mut self, font: &FontRef, text: &str, size: f64, x: f64, baseline: f64, colour: Rgb) {
        let units_per_em = font.units_per_em().unwrap_or(1000.0);
        let scale = PxScale::from(size as f32 * font.height_unscaled() / units_per_em);
        let scaled = font.as_scaled(scale);
        let mut caret = x as f32;
        let mut previous = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = previous {
                caret += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, point(caret, baseline as f32));
            if let Some(outline) = font.outline_glyph(glyph) {
                let min = outline.px_bounds().min;
                outline.draw(|gx, gy, coverage| {
                    self.blend(
                        min.x as i64 + i64::from(gx),
                        min.y as i64 + i64::from(gy),
                        colour,
                        f64::from(coverage),
                    );
                });
            }
            caret += scaled.h_advance(id);
            previous = Some(id);
        }
    }

    fn into_image(self) -> Image {
        let rgba = self
            .pixels
            .iter()
            .flat_map(|p| {
                let a = p[3];
                let straight = |c: f64| if a > 0.0 { c / a } else { 0.0 };
                [straight(p[0]), straight(p[1]), straight(p[2]), a]
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            })
            .collect();
        Image {
            width: self.width as u32,
            height: self.height as u32,
            rgba,
        }
    }
}

/// One bar of the icon, in a unit square.
struct Bar {
    rect: Rect,
    colour: Rgb,
}

/// The tile's corner radius, as a share of its side.
const TILE_RADIUS: f64 = 0.22;

/// The six bars of the icon, in a unit square: a waveform in the stem
/// colours.
fn bars() -> Vec<Bar> {
    const HEIGHTS: [f64; 6] = [0.42, 0.74, 0.56, 0.86, 0.62, 0.36];
    const PAD: f64 = 0.15;
    let slot = (1.0 - 2.0 * PAD) / 6.0;
    let width = slot * 0.72;
    STEMS
        .iter()
        .zip(HEIGHTS)
        .enumerate()
        .map(|(k, ((_, colour), height))| {
            let x0 = PAD + k as f64 * slot + (slot - width) / 2.0;
            let y0 = (1.0 - height * (1.0 - 2.0 * PAD)) / 2.0;
            Bar {
                rect: Rect {
                    x0,
                    y0,
                    x1: x0 + width,
                    y1: 1.0 - y0,
                },
                colour: *colour,
            }
        })
        .collect()
}

/// The icon at `size` px.  A rounded tile for the browser, or a square
/// one for the iPhone, which rounds the corners itself.
fn icon(size: u32, rounded: bool) -> Image {
    let s = f64::from(size);
    let mut canvas = Canvas::new(size, size);
    let tile = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: s,
        y1: s,
    };
    if rounded {
        canvas.fill_round_rect(tile, TILE_RADIUS * s, TILE);
    } else {
        canvas.fill_rect(tile, TILE, 1.0);
    }
    for bar in bars() {
        let r = Rect {
            x0: bar.rect.x0 * s,
            y0: bar.rect.y0 * s,
            x1: bar.rect.x1 * s,
            y1: bar.rect.y1 * s,
        };
        canvas.fill_round_rect(r, (r.x1 - r.x0) / 2.0, bar.colour);
    }
    canvas.into_image()
}

/// A number for SVG: at most three decimals, with no trailing zeros.
fn svg_number(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The icon as SVG, on a 32-unit grid.
pub fn icon_svg() -> String {
    const S: f64 = 32.0;
    let tile = format!(
        "<rect width=\"32\" height=\"32\" rx=\"{}\" fill=\"{}\"/>",
        svg_number(TILE_RADIUS * S),
        TILE.hex()
    );
    let bars: String = bars()
        .iter()
        .map(|b| {
            let w = (b.rect.x1 - b.rect.x0) * S;
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\"/>",
                svg_number(b.rect.x0 * S),
                svg_number(b.rect.y0 * S),
                svg_number(w),
                svg_number((b.rect.y1 - b.rect.y0) * S),
                svg_number(w / 2.0),
                b.colour.hex()
            )
        })
        .collect();
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\">{tile}{bars}</svg>\n")
}

/// A seeded generator (SplitMix64) of numbers in [0, 1).
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as f64 / (u64::MAX as f64 + 1.0)
    }
}

fn within(t: f64, ranges: &[(f64, f64)]) -> bool {
    ranges.iter().any(|&(a, b)| t >= a && t <= b)
}

/// tanh by a Padé approximant, clamped to 1: close for |x| < 3.
fn soft_clip(x: f64) -> f64 {
    (x * (27.0 + x * x) / (27.0 + 9.0 * x * x)).min(1.0)
}

/// The waveform heights of a song's lanes, from 0 to 1, over `n` columns:
/// the six stems in [`STEMS`] order, then the mix.  The sections follow
/// a real song's: a drum intro, verses, a guitar break and an outro.
fn envelopes(n: usize) -> ([Vec<f64>; 6], Vec<f64>) {
    let mut r = Random(7);
    let mut stems: [Vec<f64>; 6] = Default::default();
    let mut mix = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let bass = if !(0.07..=0.955).contains(&t) {
            0.0
        } else if r.next() < 0.08 {
            0.95
        } else {
            0.5 + 0.18 * r.next()
        };
        let drums = if !(0.02..=0.995).contains(&t) {
            0.0
        } else if t < 0.07 {
            0.25 * r.next()
        } else if r.next() < 0.35 {
            0.95
        } else {
            0.35 + 0.4 * r.next()
        };
        let other_level = if t < 0.01 {
            0.0
        } else if t < 0.21 {
            0.75
        } else if t < 0.36 {
            0.35
        } else if t < 0.57 {
            0.6
        } else if t > 0.95 && t < 0.99 {
            0.7
        } else {
            0.28
        };
        let other = other_level * (0.55 + 0.45 * r.next());
        let vocals = if within(t, &[(0.12, 0.955)]) {
            if r.next() < 0.025 {
                0.1
            } else {
                0.6 + 0.3 * r.next()
            }
        } else {
            0.0
        };
        let guitar_level = if within(t, &[(0.065, 0.12), (0.28, 0.375), (0.585, 0.955)]) {
            0.85
        } else if within(t, &[(0.38, 0.47)]) {
            0.3
        } else {
            0.0
        };
        let guitar = guitar_level * (0.8 + 0.2 * r.next());
        let phase = (t * 55.0).fract();
        let pluck = (1.0 - phase) * (1.0 - phase);
        let piano = if within(t, &[(0.07, 0.4)]) {
            0.12 * r.next()
        } else if within(t, &[(0.42, 0.95)]) {
            pluck * 0.85 * (0.4 + 0.6 * r.next())
        } else if within(t, &[(0.965, 0.99)]) {
            0.35 * r.next()
        } else {
            0.0
        };
        let levels = [bass, drums, other, vocals, guitar, piano];
        for (lane, level) in stems.iter_mut().zip(levels) {
            lane.push(level);
        }
        mix.push(soft_clip(1.1 * levels.iter().sum::<f64>()) * (0.92 + 0.08 * r.next()));
    }
    (stems, mix)
}

/// The share image: the wordmark, then a mix waveform that fans out into
/// the six stem lanes.
fn og_image() -> Result<Image> {
    let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT)
        .context("reading egui's Ubuntu-Light font")?;
    let mut canvas = Canvas::new(OG_WIDTH, OG_HEIGHT);
    canvas.fill_rect(
        Rect {
            x0: 0.0,
            y0: 0.0,
            x1: f64::from(OG_WIDTH),
            y1: f64::from(OG_HEIGHT),
        },
        OG_BACKGROUND,
        1.0,
    );
    canvas.text(&font, "stemyla", 84.0, 60.0, 112.0, WORDMARK);
    canvas.text(
        &font,
        "split any song into six stems",
        30.0,
        62.0,
        160.0,
        SUBTITLE,
    );
    canvas.text(
        &font,
        "free, private, in your browser",
        22.0,
        60.0,
        596.0,
        FOOTER,
    );

    // The mix runs from START to SPLIT, then the stems fan out over FAN px
    // to their lanes, and run on to END.
    const START: f64 = 60.0;
    const SPLIT: f64 = 470.0;
    const FAN: f64 = 260.0;
    const END: f64 = 1150.0;
    const MIX_FADE: f64 = 60.0;
    const MIX_CENTRE: f64 = 410.0;
    const MIX_HEIGHT: f64 = 190.0;
    const LANE_TOP: f64 = 222.0;
    const LANE_PITCH: f64 = 66.0;
    const LANE_HEIGHT: f64 = 52.0;
    const STEP: f64 = 3.0;
    const COLUMN: f64 = 2.2;
    const COLUMNS: usize = 400;
    let (stems, mix) = envelopes(COLUMNS);
    // The mix and the stems each show the whole song over their own span.
    let at = |lane: &[f64], x: f64, from: f64, to: f64| {
        let i = ((x - from) / (to - from) * COLUMNS as f64).floor() as usize;
        lane[i.min(COLUMNS - 1)]
    };
    let column = |x: f64, centre: f64, height: f64| Rect {
        x0: x,
        y0: centre - height / 2.0,
        x1: x + COLUMN,
        y1: centre + height / 2.0,
    };
    let mut x = START;
    while x < END {
        if x >= SPLIT {
            let f = ((x - SPLIT) / FAN).clamp(0.0, 1.0);
            let s = f * f * (3.0 - 2.0 * f);
            for (k, ((_, colour), lane)) in STEMS.iter().zip(&stems).enumerate() {
                let lane_centre = LANE_TOP + k as f64 * LANE_PITCH + LANE_HEIGHT / 2.0;
                let centre = MIX_CENTRE + (lane_centre - MIX_CENTRE) * s;
                let full = MIX_HEIGHT / 4.0 + (LANE_HEIGHT - MIX_HEIGHT / 4.0) * s;
                canvas.fill_rect(
                    column(x, centre, full * at(lane, x, SPLIT, END)),
                    *colour,
                    0.35 + 0.65 * s,
                );
            }
        }
        if x < SPLIT + MIX_FADE {
            let alpha = if x < SPLIT {
                1.0
            } else {
                1.0 - (x - SPLIT) / MIX_FADE
            };
            canvas.fill_rect(
                column(
                    x,
                    MIX_CENTRE,
                    MIX_HEIGHT * at(&mix, x, START, SPLIT + MIX_FADE),
                ),
                MIX,
                alpha,
            );
        }
        x += STEP;
    }
    Ok(canvas.into_image())
}

pub fn encode_png(image: &Image) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba)?;
    writer.finish()?;
    Ok(out)
}

/// Decodes an 8-bit RGBA PNG, as [`encode_png`] writes.
pub fn decode_png(bytes: &[u8]) -> Result<Image> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
    let size = reader
        .output_buffer_size()
        .context("the PNG is too large to decode")?;
    let mut rgba = vec![0; size];
    let info = reader.next_frame(&mut rgba)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        bail!(
            "the PNG is {:?} at {:?}, not 8-bit RGBA",
            info.color_type,
            info.bit_depth
        );
    }
    rgba.truncate(info.buffer_size());
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

const ICO_HEADER: usize = 6;
const ICO_ENTRY: usize = 16;

/// An ICO file that holds one square PNG of `size` px, as browsers read
/// since Windows Vista's format.
pub fn ico(png: &[u8], size: u32) -> Result<Vec<u8>> {
    let side = u8::try_from(size).context("an ICO entry is at most 255 px")?;
    let length = u32::try_from(png.len()).context("the PNG is too large for an ICO")?;
    let offset = (ICO_HEADER + ICO_ENTRY) as u32;
    let header = [0u16, 1, 1].iter().flat_map(|v| v.to_le_bytes());
    let entry = [side, side, 0, 0]
        .into_iter()
        .chain(1u16.to_le_bytes())
        .chain(32u16.to_le_bytes())
        .chain(length.to_le_bytes())
        .chain(offset.to_le_bytes());
    Ok(header
        .chain(entry)
        .chain(png.iter().copied())
        .collect::<Vec<u8>>())
}

/// The PNG in an ICO file of one image, as [`ico`] writes.
pub fn ico_png(ico: &[u8]) -> Result<&[u8]> {
    let u16_at = |i: usize| u16::from_le_bytes([ico[i], ico[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([ico[i], ico[i + 1], ico[i + 2], ico[i + 3]]);
    if ico.len() < ICO_HEADER + ICO_ENTRY || u16_at(0) != 0 || u16_at(2) != 1 || u16_at(4) != 1 {
        bail!("not an ICO file of one image");
    }
    let length = u32_at(ICO_HEADER + 8) as usize;
    let offset = u32_at(ICO_HEADER + 12) as usize;
    ico.get(offset..offset.saturating_add(length))
        .context("the ICO's image runs past the end of the file")
}

pub struct File {
    pub name: &'static str,
    pub bytes: Vec<u8>,
}

/// The files of [`FILES`], drawn.
pub fn files() -> Result<Vec<File>> {
    let favicon = encode_png(&icon(FAVICON_SIZE, true))?;
    Ok(vec![
        File {
            name: ICON_SVG,
            bytes: icon_svg().into_bytes(),
        },
        File {
            name: FAVICON_ICO,
            bytes: ico(&favicon, FAVICON_SIZE)?,
        },
        File {
            name: TOUCH_ICON,
            bytes: encode_png(&icon(TOUCH_ICON_SIZE, false))?,
        },
        File {
            name: OG_PNG,
            bytes: encode_png(&og_image()?)?,
        },
    ])
}

/// Whether two copies of the file `name` hold the same image.  PNGs are
/// compared by their pixels, so a new PNG encoder that compresses
/// differently does not count as a change.
pub fn same_image(name: &str, a: &[u8], b: &[u8]) -> Result<bool> {
    if name.ends_with(".png") {
        Ok(decode_png(a)? == decode_png(b)?)
    } else if name.ends_with(".ico") {
        Ok(decode_png(ico_png(a)?)? == decode_png(ico_png(b)?)?)
    } else {
        Ok(a == b)
    }
}

/// The files in `dir` that differ from what [`files`] draws, or are
/// missing, each with the reason.
pub fn stale(dir: &Path) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    for file in files()? {
        let path = dir.join(file.name);
        match std::fs::read(&path) {
            Err(e) => problems.push(format!("{}: {e}", path.display())),
            Ok(committed) => match same_image(file.name, &committed, &file.bytes) {
                Ok(true) => {}
                Ok(false) => problems.push(format!(
                    "{} differs from what `xtask brand` draws",
                    path.display()
                )),
                Err(e) => problems.push(format!("{}: {e:#}", path.display())),
            },
        }
    }
    Ok(problems)
}

pub fn main(args: &[String]) -> Result<()> {
    let dir = crate::workspace_root().join(ASSETS);
    match args {
        [] => {
            std::fs::create_dir_all(&dir)?;
            for file in files()? {
                let path = dir.join(file.name);
                std::fs::write(&path, &file.bytes)
                    .with_context(|| format!("writing {}", path.display()))?;
                println!("wrote {}", path.display());
            }
            Ok(())
        }
        [flag] if flag == "--check" => {
            let problems = stale(&dir)?;
            if problems.is_empty() {
                println!("{} matches `xtask brand`", dir.display());
                Ok(())
            } else {
                bail!(
                    "{}\nFix: run `cargo run -p xtask -- brand`, look at the files, and commit them.",
                    problems.join("\n")
                )
            }
        }
        _ => bail!("{USAGE}"),
    }
}

/// The value of the first `content="..."` after `key`, such as
/// `property="og:title"`.  The tags in `app/index.html` give the key
/// first, then `content`.
fn content_after<'a>(html: &'a str, key: &str) -> Option<&'a str> {
    let start = html.find(key)? + key.len();
    let rest = html[start..].trim_start().strip_prefix("content=\"")?;
    rest.split('"').next()
}

fn between<'a>(html: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = html.find(open)? + open.len();
    html[start..].split(close).next()
}

/// The most characters of the description that a search result shows
/// before it cuts the text, on a common desktop width.
pub const DESCRIPTION_LIMIT: usize = 160;

/// What is wrong with the head tags of `app/index.html` (`html`), given
/// the share image's size.  A value that the head repeats, such as the
/// title in `og:title`, must be equal in both places.  Each brand file must
/// be copied into the build, and each icon link must name one.
pub fn head_problems(html: &str, og_size: (u32, u32)) -> Vec<String> {
    let mut problems = Vec::new();
    let mut need = |name: &str, value: Option<&str>| -> String {
        match value {
            Some(v) if !v.trim().is_empty() => v.to_string(),
            _ => {
                problems.push(format!("the head has no {name}"));
                String::new()
            }
        }
    };
    let title = need("<title>", between(html, "<title>", "</title>"));
    let og_title = need("og:title", content_after(html, "property=\"og:title\""));
    let description = need("description", content_after(html, "name=\"description\""));
    let og_description = need(
        "og:description",
        content_after(html, "property=\"og:description\""),
    );
    let canonical = need(
        "canonical link",
        between(html, "<link rel=\"canonical\" href=\"", "\""),
    );
    let og_url = need("og:url", content_after(html, "property=\"og:url\""));
    let og_image = need("og:image", content_after(html, "property=\"og:image\""));
    let width = need(
        "og:image:width",
        content_after(html, "property=\"og:image:width\""),
    );
    let height = need(
        "og:image:height",
        content_after(html, "property=\"og:image:height\""),
    );
    need(
        "og:image:alt",
        content_after(html, "property=\"og:image:alt\""),
    );
    need("og:type", content_after(html, "property=\"og:type\""));
    need(
        "og:site_name",
        content_after(html, "property=\"og:site_name\""),
    );
    let card = need("twitter:card", content_after(html, "name=\"twitter:card\""));
    let theme = need("theme-color", content_after(html, "name=\"theme-color\""));

    if title != og_title {
        problems.push(format!("og:title {og_title:?} is not the title {title:?}"));
    }
    if description != og_description {
        problems.push(format!(
            "og:description {og_description:?} is not the description {description:?}"
        ));
    }
    if description.chars().count() > DESCRIPTION_LIMIT {
        problems.push(format!(
            "the description has {} characters; search results cut it after about {DESCRIPTION_LIMIT}",
            description.chars().count()
        ));
    }
    if !canonical.starts_with("https://") || !canonical.ends_with('/') {
        problems.push(format!(
            "the canonical URL {canonical:?} is not an absolute https:// URL of a folder"
        ));
    }
    if og_url != canonical {
        problems.push(format!(
            "og:url {og_url:?} is not the canonical URL {canonical:?}"
        ));
    }
    if og_image != format!("{canonical}{OG_PNG}") {
        problems.push(format!(
            "og:image {og_image:?} is not {OG_PNG} at the canonical URL, where `xtask publish` puts it"
        ));
    }
    if (width.as_str(), height.as_str())
        != (
            og_size.0.to_string().as_str(),
            og_size.1.to_string().as_str(),
        )
    {
        problems.push(format!(
            "og:image is {width} x {height} in the head, but {OG_PNG} is {} x {}",
            og_size.0, og_size.1
        ));
    }
    if card != "summary_large_image" {
        problems.push(format!(
            "twitter:card is {card:?}, not \"summary_large_image\""
        ));
    }
    if theme != TILE.hex() {
        problems.push(format!(
            "theme-color is {theme:?}, not the tile's {}",
            TILE.hex()
        ));
    }
    for name in FILES {
        if !html.contains(&format!(
            "<link data-trunk rel=\"copy-file\" href=\"assets/{name}\""
        )) {
            problems.push(format!(
                "the head does not copy assets/{name} into the build"
            ));
        }
    }
    for (rel, name) in [
        ("icon", FAVICON_ICO),
        ("icon", ICON_SVG),
        ("apple-touch-icon", TOUCH_ICON),
    ] {
        if !html.contains(&format!("<link rel=\"{rel}\" href=\"{name}\"")) {
            problems.push(format!(
                "the head has no <link rel=\"{rel}\" href=\"{name}\">"
            ));
        }
    }
    problems
}
