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
    /// The per-type default (prettify JSON, copy text, reveal path...).
    Auto,
}

/// Size target of the "Make under N MB" chip.
pub const TARGET_MB: u64 = 5;

/// Chips offered for a shelf item, in order.
pub fn chips_for(path: &Path) -> Vec<(&'static str, Conv)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => vec![
            ("Under 5 MB", Conv::Under(TARGET_MB)),
            ("To PDF", Conv::Pdf),
            ("Remove location", Conv::Clean),
            ("Read text", Conv::Ocr),
        ],
        "webp" | "bmp" => vec![("Under 5 MB", Conv::Under(TARGET_MB)), ("To PDF", Conv::Pdf), ("Read text", Conv::Ocr)],
        "json" | "yaml" | "yml" => vec![("Format", Conv::Auto)],
        "txt" | "md" => vec![("Copy text", Conv::Auto)],
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
        Conv::Pdf => done(convert::to_pdf(&[src.to_path_buf()], |s, e| place(src, s, e)), "PDF"),
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
        Conv::Auto => transform_file(src),
    }
}

/// Decode + re-encode drops EXIF/GPS.
fn clean_image(src: &Path) -> Outcome {
    let img = match image::open(src) {
        Ok(i) => i,
        Err(e) => return Outcome::msg(format!("Can't read image: {e}")),
    };
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let ext = if ext == "jpeg" { "jpg".to_string() } else { ext };
    let clean = place(src, ".clean", &ext);
    let saved = if ext == "jpg" { image::DynamicImage::ImageRgb8(img.to_rgb8()).save(&clean) } else { img.save(&clean) };
    match saved {
        Ok(_) => Outcome { summary: "Location removed".into(), open: Some(clean), copy: None },
        Err(e) => Outcome::msg(format!("Write failed: {e}")),
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
        Self { summary: s.into(), ..Default::default() }
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
        Ok(out) => Outcome { summary: "JSON formatted · copied".into(), open: None, copy: Some(out) },
        Err(_) => Outcome {
            summary: format!("Text copied · {} chars", text.trim().chars().count()),
            open: None,
            copy: Some(text.trim().to_string()),
        },
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string()
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
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
        let ext = if ext == "jpeg" { "jpg".to_string() } else { ext };
        let clean = place(src, ".clean", &ext);
        let saved = if ext == "jpg" {
            image::DynamicImage::ImageRgb8(img.to_rgb8()).save(&clean)
        } else {
            img.save(&clean)
        };
        if saved.is_ok() {
            parts.push("EXIF stripped".into());
            open = Some(clean);
        }
    }
    let is_webp = src.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("webp"));
    if !is_webp {
        let webp = place(src, "", "webp");
        // image-webp encodes lossless only.
        if image::DynamicImage::ImageRgba8(img.to_rgba8()).save(&webp).is_ok() {
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
    Outcome { summary: parts.join(" · "), open, copy }
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
    let file = block_on(StorageFile::GetFileFromPathAsync(&HSTRING::from(p)).map_err(e("open"))?, T)
        .ok_or("timeout")?
        .map_err(e("open"))?;
    let stream = block_on(file.OpenAsync(FileAccessMode::Read).map_err(e("stream"))?, T)
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
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
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
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255])).save(&src).unwrap();
        let out = transform_image(&src, true);
        assert!(out.summary.contains("EXIF stripped"), "{}", out.summary);
        assert!(out.summary.contains("WebP"), "{}", out.summary);
    }

    #[test]
    fn chips_follow_file_type_and_actions_are_separate() {
        let labels = |f: &str| chips_for(Path::new(f)).iter().map(|c| c.0).collect::<Vec<_>>();
        assert_eq!(labels("a.JPG"), ["Under 5 MB", "To PDF", "Remove location", "Read text"]);
        assert_eq!(labels("a.json"), ["Format"]);
        assert_eq!(labels("a.zip"), ["Copy path"]);
        // "Remove location" writes only the cleaned copy: no WebP or OCR side effects.
        let dir = std::env::temp_dir().join("hytte-chip");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("c.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255])).save(&src).unwrap();
        let o = run(Conv::Clean, &src);
        assert_eq!(o.summary, "Location removed");
        assert!(o.copy.is_none());
    }
}
