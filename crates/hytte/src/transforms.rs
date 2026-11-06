//! Drop Vault transforms v1:
//! EXIF strip, lossless WebP, JSON/YAML prettify/minify, OCR→clipboard.
//! Outputs are written next to the source (or the configured folder) with a
//! suffix, never overwriting.

use std::path::{Path, PathBuf};

/// One explicit shelf action (a chip), instead of "do everything" on click.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Conv {
    /// Re-encode as JPEG no larger than this many MB.
    Under(u64),
    Pdf,
    /// Re-encode to drop EXIF/GPS.
    Clean,
    Ocr,
    /// Decode a text snippet that is a JWT, a Unix time, URL- or base64-encoded, or a hex
    /// colour (see [`decode`]).
    Decode,
    /// Turn a text snippet (a link, a Wi-Fi code, a note) into a QR code image to scan.
    Qr,
    /// The per-type default (prettify JSON, copy text, reveal path...).
    Auto,
}

/// Size target of the "Make under N MB" chip.
pub const TARGET_MB: u64 = 5;

/// Chips offered for a shelf item, in order.
pub fn chips_for(path: &Path) -> Vec<(&'static str, Conv)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => vec![
            ("Compress", Conv::Under(TARGET_MB)),
            ("To PDF", Conv::Pdf),
            ("Remove metadata", Conv::Clean),
            ("Read text", Conv::Ocr),
        ],
        "webp" | "bmp" => vec![
            ("Compress", Conv::Under(TARGET_MB)),
            ("To PDF", Conv::Pdf),
            ("Read text", Conv::Ocr),
        ],
        "json" | "yaml" | "yml" => vec![("Format", Conv::Auto)],
        "txt" => match decode_label(path) {
            Some(l) => vec![
                (l, Conv::Decode),
                ("Copy text", Conv::Auto),
                ("QR code", Conv::Qr),
            ],
            None => vec![("Copy text", Conv::Auto), ("QR code", Conv::Qr)],
        },
        "md" => vec![("Copy text", Conv::Auto)],
        _ => vec![("Copy path", Conv::Auto)],
    }
}

pub fn run(op: Conv, src: &Path) -> Outcome {
    use crate::convert::{self, fmt_size};
    let done = |r: Result<convert::Converted, String>, verb: &str| match r {
        Ok(c) => Outcome {
            summary: if verb.is_empty() {
                format!("{} → {}", fmt_size(c.before), fmt_size(c.after))
            } else {
                format!("{verb} · {}", fmt_size(c.after))
            },
            open: Some(c.path),
            copy: None,
        },
        Err(e) => Outcome::msg(e),
    };
    match op {
        Conv::Under(mb) => done(convert::compress(src, mb, |s, e| place(src, s, e)), ""),
        Conv::Pdf => done(
            convert::to_pdf(&[src.to_path_buf()], |s, e| place(src, s, e)),
            "PDF",
        ),
        Conv::Clean => clean_image(src),
        Conv::Ocr => match ocr_image(src) {
            Ok(t) if !t.trim().is_empty() => Outcome {
                summary: format!("{} characters copied", t.trim().chars().count()),
                open: None,
                copy: Some(t.trim().to_string()),
            },
            Ok(_) => Outcome::msg("No text found"),
            Err(e) => Outcome::msg(format!("Can't read text ({e})")),
        },
        Conv::Decode => match std::fs::read_to_string(src)
            .ok()
            .as_deref()
            .and_then(decode)
        {
            Some((_, out, summary)) => Outcome {
                summary,
                open: None,
                copy: Some(out),
            },
            None => Outcome::msg("Nothing to decode"),
        },
        Conv::Qr => match std::fs::read_to_string(src) {
            Ok(text) => match qr_png(text.trim()) {
                Ok(img) => {
                    let out = place(src, ".qr", "png");
                    match img.save(&out) {
                        Ok(()) => Outcome {
                            summary: "QR code ready; scan it from the screen".into(),
                            open: Some(out),
                            copy: None,
                        },
                        Err(e) => Outcome::msg(format!("Write failed: {e}")),
                    }
                }
                Err(e) => Outcome::msg(e),
            },
            Err(e) => Outcome::msg(format!("Can't read the text: {e}")),
        },
        Conv::Auto => transform_file(src),
    }
}

/// A QR code for `text` as a black-on-white image: 8 px per module with the 4-module quiet
/// zone scanners need.
pub fn qr_png(text: &str) -> Result<image::GrayImage, String> {
    use qrcodegen::{QrCode, QrCodeEcc};
    if text.is_empty() {
        return Err("Nothing to encode".into());
    }
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium)
        .map_err(|_| "Too long for a QR code (about 2,300 characters at most)".to_string())?;
    const SCALE: u32 = 8;
    const QUIET: u32 = 4;
    let n = qr.size() as u32;
    let side = (n + 2 * QUIET) * SCALE;
    let mut img = image::GrayImage::from_pixel(side, side, image::Luma([255]));
    for y in 0..n {
        for x in 0..n {
            if qr.get_module(x as i32, y as i32) {
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        img.put_pixel(
                            (x + QUIET) * SCALE + dx,
                            (y + QUIET) * SCALE + dy,
                            image::Luma([0]),
                        );
                    }
                }
            }
        }
    }
    Ok(img)
}

/// Decode + re-encode drops EXIF/GPS.
fn clean_image(src: &Path) -> Outcome {
    let img = match image::open(src) {
        Ok(i) => i,
        Err(e) => return Outcome::msg(format!("Can't read image: {e}")),
    };
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let ext = if ext == "jpeg" {
        "jpg".to_string()
    } else {
        ext
    };
    let clean = place(src, ".clean", &ext);
    match save_clean(&img, &clean, &ext) {
        Ok(_) => Outcome {
            summary: "Location removed".into(),
            open: Some(clean),
            copy: None,
        },
        Err(e) => Outcome::msg(format!("Write failed: {e}")),
    }
}

/// Re-encode without metadata. JPEG needs RGB8; an image already in that layout
/// (the usual case for photos) is saved without a full-size copy.
fn save_clean(img: &image::DynamicImage, dst: &Path, ext: &str) -> image::ImageResult<()> {
    match img {
        image::DynamicImage::ImageRgb8(_) => img.save(dst),
        _ if ext == "jpg" => image::DynamicImage::ImageRgb8(img.to_rgb8()).save(dst),
        _ => img.save(dst),
    }
}

/// What a transform produced, for the result chip.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub summary: String,
    pub open: Option<PathBuf>,
    pub copy: Option<String>,
}

impl Outcome {
    fn msg(s: impl Into<String>) -> Self {
        Self {
            summary: s.into(),
            ..Default::default()
        }
    }
}

pub fn transform_file(path: &Path) -> Outcome {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => transform_image(path, true),
        "webp" | "bmp" => transform_image(path, false),
        "json" => transform_json(path),
        "yaml" | "yml" => transform_yaml(path),
        "txt" | "md" => transform_text(path),
        _ => {
            #[cfg(windows)]
            reveal_in_explorer(path);
            Outcome {
                summary: format!("Path copied · {}", file_name(path)),
                open: Some(path.to_path_buf()),
                copy: Some(path.display().to_string()),
            }
        }
    }
}

/// Shelved text snippet: copy it, formatting JSON on the way.
fn transform_text(src: &Path) -> Outcome {
    let text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(e) => return Outcome::msg(format!("Can't read file: {e}")),
    };
    match toggle_json(&text) {
        Ok(out) => Outcome {
            summary: "JSON formatted · copied".into(),
            open: None,
            copy: Some(out),
        },
        Err(_) => Outcome {
            summary: format!("Text copied · {} chars", text.trim().chars().count()),
            open: None,
            copy: Some(text.trim().to_string()),
        },
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string()
}

fn unique_in(dir: &Path, stem: &str, suffix: &str, ext: &str) -> PathBuf {
    let mut i = 0;
    loop {
        let name = if i == 0 {
            format!("{stem}{suffix}.{ext}")
        } else {
            format!("{stem}{suffix}-{i}.{ext}")
        };
        let cand = dir.join(name);
        if !cand.exists() {
            return cand;
        }
        i += 1;
    }
}

fn unique_next_to(src: &Path, suffix: &str, ext: &str) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    unique_in(src.parent().unwrap_or(Path::new(".")), stem, suffix, ext)
}

/// Output path: configured folder when valid, else beside the source.
fn place(src: &Path, suffix: &str, ext: &str) -> PathBuf {
    let folder = crate::config::load().general.output_folder;
    match folder.map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(dir) => {
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
            unique_in(&dir, stem, suffix, ext)
        }
        None => unique_next_to(src, suffix, ext),
    }
}

/// Image: decode + re-encode drops EXIF/GPS; also lossless WebP; OCR → clipboard.
fn transform_image(src: &Path, strip: bool) -> Outcome {
    let img = match image::open(src) {
        Ok(i) => i,
        Err(e) => return Outcome::msg(format!("Can't read image: {e}")),
    };
    let mut parts: Vec<String> = vec![];
    let mut open = None;
    if strip {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let ext = if ext == "jpeg" {
            "jpg".to_string()
        } else {
            ext
        };
        let clean = place(src, ".clean", &ext);
        if save_clean(&img, &clean, &ext).is_ok() {
            parts.push("EXIF stripped".into());
            open = Some(clean);
        }
    }
    let is_webp = src
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webp"));
    if !is_webp {
        let webp = place(src, "", "webp");
        // image-webp encodes lossless only. RGBA8 is saved as is; anything else is converted.
        let saved = match &img {
            image::DynamicImage::ImageRgba8(_) => img.save(&webp),
            _ => image::DynamicImage::ImageRgba8(img.to_rgba8()).save(&webp),
        };
        if saved.is_ok() {
            parts.push("WebP (lossless)".into());
            open = Some(webp);
        }
    }
    let mut copy = None;
    match ocr_image(src) {
        Ok(t) if !t.trim().is_empty() => {
            parts.push(format!("OCR {} chars copied", t.trim().chars().count()));
            copy = Some(t.trim().to_string());
        }
        Ok(_) => parts.push("OCR: no text".into()),
        Err(e) => parts.push(format!("OCR unavailable ({e})")),
    }
    Outcome {
        summary: parts.join(" · "),
        open,
        copy,
    }
}

/// Pretty if the input is a single line, minified otherwise.
pub fn toggle_json(text: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| e.to_string())?;
    let r = if text.trim().contains('\n') {
        serde_json::to_string(&v)
    } else {
        serde_json::to_string_pretty(&v)
    };
    r.map_err(|e| e.to_string())
}

fn transform_json(src: &Path) -> Outcome {
    let text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(e) => return Outcome::msg(format!("Can't read file: {e}")),
    };
    match toggle_json(&text) {
        Ok(out) => {
            let minified = text.trim().contains('\n');
            let dst = place(src, if minified { ".min" } else { ".pretty" }, "json");
            match std::fs::write(&dst, &out) {
                Ok(_) => Outcome {
                    summary: format!(
                        "JSON {} · {}",
                        if minified { "minified" } else { "prettified" },
                        file_name(&dst)
                    ),
                    open: Some(dst),
                    copy: Some(out),
                },
                Err(e) => Outcome::msg(format!("Write failed: {e}")),
            }
        }
        Err(e) => Outcome::msg(format!("Invalid JSON: {e}")),
    }
}

/// No YAML parser dependency: normalise whitespace only (documented limit).
pub fn normalize_yaml(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for l in text.lines() {
        let l = l.trim_end().replace('\t', "  ");
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(&l);
        out.push('\n');
    }
    out.trim_end().to_string() + "\n"
}

fn transform_yaml(src: &Path) -> Outcome {
    let text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(e) => return Outcome::msg(format!("Can't read file: {e}")),
    };
    let out = normalize_yaml(&text);
    let dst = place(src, ".pretty", "yaml");
    match std::fs::write(&dst, &out) {
        Ok(_) => Outcome {
            summary: format!("YAML normalised · {}", file_name(&dst)),
            open: Some(dst),
            copy: Some(out),
        },
        Err(e) => Outcome::msg(format!("Write failed: {e}")),
    }
}

/// OCR via Windows.Media.Ocr using the user's installed languages.
pub fn ocr_image(src: &Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        ocr_windows(src)
    }
    #[cfg(not(windows))]
    {
        let _ = src;
        Err("not supported on this platform".into())
    }
}

#[cfg(windows)]
fn ocr_windows(src: &Path) -> Result<String, String> {
    use crate::winrt::block_on;
    use std::time::Duration;
    use windows::core::HSTRING;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};
    const T: Duration = Duration::from_secs(15);
    fn e(what: &'static str) -> impl Fn(windows::core::Error) -> String {
        move |x| format!("{what}: {x}")
    }

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| "no OCR language pack installed".to_string())?;
    let abs = std::fs::canonicalize(src).map_err(|x| x.to_string())?;
    let mut p = abs.display().to_string();
    if let Some(stripped) = p.strip_prefix(r"\\?\") {
        p = stripped.to_string();
    }
    let file = block_on(
        StorageFile::GetFileFromPathAsync(&HSTRING::from(p)).map_err(e("open"))?,
        T,
    )
    .ok_or("timeout")?
    .map_err(e("open"))?;
    let stream = block_on(
        file.OpenAsync(FileAccessMode::Read).map_err(e("stream"))?,
        T,
    )
    .ok_or("timeout")?
    .map_err(e("stream"))?;
    let decoder = block_on(BitmapDecoder::CreateAsync(&stream).map_err(e("decode"))?, T)
        .ok_or("timeout")?
        .map_err(e("decode"))?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(4096);
    if decoder.PixelWidth().unwrap_or(0) > max || decoder.PixelHeight().unwrap_or(0) > max {
        return Err(format!("image larger than {max}px"));
    }
    let bmp = block_on(decoder.GetSoftwareBitmapAsync().map_err(e("bitmap"))?, T)
        .ok_or("timeout")?
        .map_err(e("bitmap"))?;
    let res = block_on(engine.RecognizeAsync(&bmp).map_err(e("recognize"))?, T)
        .ok_or("timeout")?
        .map_err(e("recognize"))?;
    Ok(res.Text().map_err(e("text"))?.to_string())
}

#[cfg(windows)]
fn reveal_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

/// The Decode chip's label for a text snippet, or None. Render asks every frame while the
/// item is selected, so the last answer is kept.
// ponytail: one-entry cache; only the selected shelf item is ever asked
fn decode_label(path: &Path) -> Option<&'static str> {
    use std::cell::RefCell;
    thread_local! {
        static LAST: RefCell<Option<(PathBuf, Option<&'static str>)>> = const { RefCell::new(None) };
    }
    LAST.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, l)) = c.as_ref().filter(|(p, _)| p == path) {
            return *l;
        }
        let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= 64 * 1024);
        let l = small
            .then(|| std::fs::read_to_string(path).ok())
            .flatten()
            .and_then(|t| decode(&t).map(|d| d.0));
        *c = Some((path.to_path_buf(), l));
        l
    })
}

/// What a text snippet decodes to, if it is one of the things developers paste around:
/// (chip label, text to copy, card summary).
pub fn decode(text: &str) -> Option<(&'static str, String, String)> {
    let t = text.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    if let Some(rgb) = hex_colour(t) {
        return Some(("To RGB", rgb.clone(), format!("{rgb} · copied")));
    }
    if let Some(ms) = unix_ms(t) {
        let d = local_datetime(ms)?;
        return Some(("To date", d.clone(), format!("{d} · copied")));
    }
    if let Some((json, exp)) = jwt(t) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let summary = match exp.and_then(|e| local_datetime(e * 1000).map(|d| (e, d))) {
            Some((e, d)) if e < now => format!("JWT · expired {d} · copied"),
            Some((_, d)) => format!("JWT · expires {d} · copied"),
            None => "JWT decoded · copied".into(),
        };
        return Some(("Decode JWT", json, summary));
    }
    if let Some(u) = url_decode(t) {
        return Some(("URL decode", u, "URL decoded · copied".into()));
    }
    if t.len() >= 8 {
        if let Some(b) = b64(t).and_then(|b| String::from_utf8(b).ok()) {
            if !b.is_empty() && b.chars().all(|c| !c.is_control() || "\n\r\t".contains(c)) {
                return Some(("Base64 decode", b, "Base64 decoded · copied".into()));
            }
        }
    }
    None
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` → `rgb(r, g, b)` (`rgba` with alpha).
fn hex_colour(t: &str) -> Option<String> {
    let h = t.strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => h.into(),
        _ => return None,
    };
    let v: Vec<u8> = (0..full.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&full[i..i + 2], 16).unwrap_or(0))
        .collect();
    Some(match v[..] {
        [r, g, b, a] => format!("rgba({r}, {g}, {b}, {:.2})", a as f32 / 255.0),
        _ => format!("rgb({}, {}, {})", v[0], v[1], v[2]),
    })
}

/// 10 digits (seconds) or 13 (milliseconds) between 2001 and 2100, as Unix ms.
fn unix_ms(t: &str) -> Option<u64> {
    if !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = t.parse().ok()?;
    let ms = match t.len() {
        10 => n * 1000,
        13 => n,
        _ => return None,
    };
    (978_307_200_000..4_102_444_800_000)
        .contains(&ms)
        .then_some(ms)
}

/// A JWT's header and payload as pretty JSON, and its `exp` (Unix seconds).
fn jwt(t: &str) -> Option<(String, Option<u64>)> {
    let mut parts = t.split('.');
    let (h, p, _sig) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let json = |s: &str| serde_json::from_slice::<serde_json::Value>(&b64(s)?).ok();
    let (header, payload) = (json(h)?, json(p)?);
    header.get("alg")?;
    let exp = payload.get("exp").and_then(|e| e.as_u64());
    let out = serde_json::json!({ "header": header, "payload": payload });
    Some((serde_json::to_string_pretty(&out).ok()?, exp))
}

/// Text with at least one `%XX` escape, decoded (must be UTF-8).
fn url_decode(t: &str) -> Option<String> {
    let b = t.as_bytes();
    let hex = |i: usize| u8::from_str_radix(t.get(i..i + 2)?, 16).ok();
    let (mut out, mut i, mut any) = (Vec::with_capacity(b.len()), 0, false);
    while i < b.len() {
        match (b[i], hex(i + 1)) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
                any = true;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    any.then(|| String::from_utf8(out).ok()).flatten()
}

/// Standard or URL-safe base64, padding optional.
fn b64(t: &str) -> Option<Vec<u8>> {
    let t = t.trim_end_matches('=');
    if t.len() % 4 == 1 {
        return None;
    }
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(t.len() * 3 / 4), 0u32, 0);
    for c in t.bytes() {
        acc = (acc << 6) | val(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Unix ms as the user's local "YYYY-MM-DD HH:MM:SS".
fn local_datetime(unix_ms: u64) -> Option<String> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
        use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
        let ft = unix_ms * 10_000 + 116_444_736_000_000_000; // 100 ns ticks since 1601
        let ft = FILETIME {
            dwLowDateTime: ft as u32,
            dwHighDateTime: (ft >> 32) as u32,
        };
        let (mut utc, mut l) = (SYSTEMTIME::default(), SYSTEMTIME::default());
        unsafe {
            FileTimeToSystemTime(&ft, &mut utc).ok()?;
            SystemTimeToTzSpecificLocalTime(None, &utc, &mut l).ok()?;
        }
        Some(format!(
            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
            l.wYear, l.wMonth, l.wDay, l.wHour, l.wMinute, l.wSecond
        ))
    }
    #[cfg(not(windows))]
    {
        let _ = unix_ms;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_never_overwrites() {
        let dir = std::env::temp_dir().join("hytte-unique");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("a.json");
        std::fs::write(&src, "{}").unwrap();
        let first = unique_next_to(&src, ".pretty", "json");
        std::fs::write(&first, "x").unwrap();
        let second = unique_next_to(&src, ".pretty", "json");
        assert_ne!(first, second);
    }

    #[test]
    fn snippets_decode_by_what_they_are() {
        let d = |t: &str| decode(t).map(|d| (d.0, d.1));
        assert_eq!(d("#f80"), Some(("To RGB", "rgb(255, 136, 0)".into())));
        assert_eq!(
            d(" #11223380 "),
            Some(("To RGB", "rgba(17, 34, 51, 0.50)".into()))
        );
        let (l, date) = d("1700000000").unwrap();
        assert_eq!((l, date.len()), ("To date", 19));
        assert!(date.starts_with("2023-11-1"), "{date}");
        assert_eq!(d("1700000000000").unwrap().1, date);
        // {"alg":"HS256"}.{"sub":"1","exp":1700000000}.sig
        let tok = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIiwiZXhwIjoxNzAwMDAwMDAwfQ.c2ln";
        let (l, json, summary) = decode(tok).unwrap();
        assert_eq!(l, "Decode JWT");
        assert!(json.contains("\"sub\": \"1\""), "{json}");
        assert!(summary.starts_with("JWT · expired 2023-11-1"), "{summary}");
        assert_eq!(d("a%20b%2Fc"), Some(("URL decode", "a b/c".into())));
        assert_eq!(
            d("aGVsbG8gd29ybGQ="),
            Some(("Base64 decode", "hello world".into()))
        );
        // Plain words, sentences, other numbers and a stray % stay plain text.
        for t in [
            "password",
            "hello there",
            "12345",
            "#zzz",
            "100%",
            "999999999999",
        ] {
            assert_eq!(d(t), None, "{t}");
        }
        assert_eq!(
            labels_of("snippet.txt", "#fff"),
            ["To RGB", "Copy text", "QR code"]
        );
        assert_eq!(
            labels_of("note.txt", "just words"),
            ["Copy text", "QR code"]
        );
    }

    fn labels_of(name: &str, text: &str) -> Vec<&'static str> {
        let dir = std::env::temp_dir().join("hytte-decode");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join(name);
        std::fs::write(&f, text).unwrap();
        chips_for(&f).iter().map(|c| c.0).collect()
    }

    #[test]
    fn json_toggles() {
        let pretty = toggle_json(r#"{"a":1,"b":[1,2]}"#).unwrap();
        assert!(pretty.contains('\n'));
        assert_eq!(toggle_json(&pretty).unwrap(), r#"{"a":1,"b":[1,2]}"#);
        assert!(toggle_json("{nope").is_err());
    }

    #[test]
    fn yaml_normalises() {
        assert_eq!(normalize_yaml("a: 1  \n\n\n\tb: 2\n\n"), "a: 1\n\n  b: 2\n");
    }

    #[test]
    fn image_clean_and_webp() {
        let dir = std::env::temp_dir().join("hytte-img");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("t.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]))
            .save(&src)
            .unwrap();
        let out = transform_image(&src, true);
        assert!(out.summary.contains("EXIF stripped"), "{}", out.summary);
        assert!(out.summary.contains("WebP"), "{}", out.summary);
    }

    #[test]
    fn chips_follow_file_type_and_actions_are_separate() {
        let labels = |f: &str| {
            chips_for(Path::new(f))
                .iter()
                .map(|c| c.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels("a.JPG"),
            ["Compress", "To PDF", "Remove metadata", "Read text"]
        );
        assert_eq!(labels("a.json"), ["Format"]);
        assert_eq!(labels("a.zip"), ["Copy path"]);
        // "Remove metadata" writes only the cleaned copy: no WebP or OCR side effects.
        let dir = std::env::temp_dir().join("hytte-chip");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("c.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]))
            .save(&src)
            .unwrap();
        let o = run(Conv::Clean, &src);
        assert_eq!(o.summary, "Location removed");
        assert!(o.copy.is_none());
    }

    #[test]
    fn qr_codes_have_a_quiet_zone_and_refuse_overlong_text() {
        let img = qr_png("https://example.com/a?b=c").unwrap();
        let (w, h) = img.dimensions();
        assert_eq!(w, h);
        assert_eq!(w % 8, 0);
        // The quiet zone (4 modules = 32 px) is white all round, and there is ink inside.
        assert!(img.pixels().take(w as usize * 32).all(|p| p.0[0] == 255));
        assert!(img.pixels().any(|p| p.0[0] == 0));
        assert!(qr_png("").is_err());
        let long = qr_png(&"x".repeat(5000)).unwrap_err();
        assert!(long.contains("Too long"));
        // The snippet chip row offers it.
        assert!(chips_for(Path::new("a.txt"))
            .iter()
            .any(|c| c.0 == "QR code"));
    }
}
