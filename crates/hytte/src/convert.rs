//! Shelf conversions for non-technical users: "make under N MB" and "to PDF".
//! Pure functions over bytes/paths; the UI wiring lives elsewhere. No PDF library:
//! a PDF with one image per page is a few dozen lines of text and offsets.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, RgbImage};
use std::path::{Path, PathBuf};

/// Refuse images larger than this many pixels (RGBA decode is 4 bytes/px).
const MAX_PIXELS: u64 = 60_000_000;
const MIN_QUALITY: u8 = 45;
const START_QUALITY: u8 = 90;

/// What a conversion produced, for the result chip.
pub struct Converted {
    pub path: PathBuf,
    pub before: u64,
    pub after: u64,
}

pub fn fmt_size(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", b.div_ceil(1024))
    }
}

fn open(path: &Path) -> Result<DynamicImage, String> {
    let (w, h) = image::image_dimensions(path).map_err(|e| format!("Can't read image: {e}"))?;
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("Image too large ({w}x{h})"));
    }
    image::open(path).map_err(|e| format!("Can't read image: {e}"))
}

/// Composite onto white so transparent PNGs do not turn black as JPEG.
fn flatten(img: &DynamicImage) -> RgbImage {
    let rgba = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
        let a = p[3] as u32;
        for c in 0..3 {
            o[c] = ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

fn jpeg(img: &RgbImage, q: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    // Encoding into a Vec cannot fail for a valid RGB buffer.
    let _ = JpegEncoder::new_with_quality(&mut buf, q).encode(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgb8);
    buf
}

/// Highest JPEG quality in `MIN_QUALITY..=START_QUALITY` that fits, by bisection (<= 6 encodes).
fn best_quality(img: &RgbImage, max: u64) -> Option<Vec<u8>> {
    let top = jpeg(img, START_QUALITY);
    if top.len() as u64 <= max {
        return Some(top);
    }
    if jpeg(img, MIN_QUALITY).len() as u64 > max {
        return None;
    }
    let (mut lo, mut hi) = (MIN_QUALITY, START_QUALITY); // lo fits, hi does not
    let mut best = None;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let b = jpeg(img, mid);
        if b.len() as u64 <= max {
            lo = mid;
            best = Some(b);
        } else {
            hi = mid;
        }
    }
    best.or_else(|| Some(jpeg(img, lo)))
}

/// Encode as JPEG no larger than `max` bytes: lower quality first, then shrink the image.
pub fn fit_jpeg(img: &DynamicImage, max: u64) -> Option<Vec<u8>> {
    let mut rgb = flatten(img);
    for _ in 0..4 {
        if let Some(b) = best_quality(&rgb, max) {
            return Some(b);
        }
        // Even the lowest quality is too big: scale by the area ratio, with headroom.
        let size = jpeg(&rgb, MIN_QUALITY).len() as f64;
        let f = (max as f64 / size).sqrt() * 0.95;
        let (w, h) = (((rgb.width() as f64 * f) as u32).max(1), ((rgb.height() as f64 * f) as u32).max(1));
        rgb = image::imageops::resize(&rgb, w, h, FilterType::Triangle);
    }
    None
}

/// "Make under N MB". Writes `<name>-<N>mb.jpg` beside the source (or in the output folder).
pub fn compress(src: &Path, max_mb: u64, dst: impl FnOnce(&str, &str) -> PathBuf) -> Result<Converted, String> {
    let before = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    let max = max_mb << 20;
    if before <= max {
        return Err(format!("Already under {max_mb} MB ({})", fmt_size(before)));
    }
    let img = open(src)?;
    let bytes = fit_jpeg(&img, max).ok_or("Couldn't get it that small")?;
    let path = dst(&format!("-{max_mb}mb"), "jpg");
    std::fs::write(&path, &bytes).map_err(|e| format!("Write failed: {e}"))?;
    Ok(Converted { path, before, after: bytes.len() as u64 })
}

// ───────────────────────────── PDF ─────────────────────────────

pub struct PdfPage {
    pub w: u32,
    pub h: u32,
    /// DeviceRGB or DeviceGray.
    pub gray: bool,
    /// `true` = `data` is a JPEG (DCTDecode); `false` = zlib-compressed raw samples.
    pub dct: bool,
    pub data: Vec<u8>,
}

const A4: (f64, f64) = (595.0, 842.0);
const MARGIN: f64 = 24.0;

/// Page box (pt) and image placement (x, y, w, h) for an image of `w` x `h` px, fitted and centred.
fn place(w: u32, h: u32) -> ((f64, f64), (f64, f64, f64, f64)) {
    let page = if w > h { (A4.1, A4.0) } else { A4 };
    let s = ((page.0 - 2.0 * MARGIN) / w as f64).min((page.1 - 2.0 * MARGIN) / h as f64);
    let (iw, ih) = (w as f64 * s, h as f64 * s);
    (page, ((page.0 - iw) / 2.0, (page.1 - ih) / 2.0, iw, ih))
}

pub fn pdf_from_pages(pages: &[PdfPage]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offs: Vec<usize> = Vec::new(); // object n (1-based) starts at offs[n-1]
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offs.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // 1 = catalog, 2 = pages, then (page, content, image) per page.
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: String = (0..pages.len()).map(|i| format!("{} 0 R ", 3 + i * 3)).collect();
    obj(&mut out, format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).as_bytes());
    for (i, p) in pages.iter().enumerate() {
        let (pg, (x, y, w, h)) = place(p.w, p.h);
        let n = 3 + i * 3;
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                pg.0,
                pg.1,
                n + 2,
                n + 1
            )
            .as_bytes(),
        );
        let content = format!("q {w:.2} 0 0 {h:.2} {x:.2} {y:.2} cm /Im0 Do Q");
        obj(&mut out, format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).as_bytes());
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{} /BitsPerComponent 8 /Filter /{} /Length {} >>\nstream\n",
            p.w,
            p.h,
            if p.gray { "DeviceGray" } else { "DeviceRGB" },
            if p.dct { "DCTDecode" } else { "FlateDecode" },
            p.data.len()
        )
        .into_bytes();
        img.extend_from_slice(&p.data);
        img.extend_from_slice(b"\nendstream");
        obj(&mut out, &img);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offs.len() + 1).as_bytes());
    for o in &offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offs.len() + 1).as_bytes(),
    );
    out
}

fn page_for(src: &Path) -> Result<PdfPage, String> {
    let img = open(src)?;
    let (w, h) = (img.width(), img.height());
    let is_jpeg = src
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg"));
    let gray = img.color().channel_count() <= 2;
    if is_jpeg {
        // Pass the original bytes through: no recompression, no quality loss.
        return Ok(PdfPage { w, h, gray, dct: true, data: std::fs::read(src).map_err(|e| e.to_string())? });
    }
    let raw = flatten(&img).into_raw();
    Ok(PdfPage { w, h, gray: false, dct: false, data: miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6) })
}

/// One page per image, in order. Output name comes from `dst`.
pub fn to_pdf(srcs: &[PathBuf], dst: impl FnOnce(&str, &str) -> PathBuf) -> Result<Converted, String> {
    if srcs.is_empty() {
        return Err("Nothing to convert".into());
    }
    let pages = srcs.iter().map(|p| page_for(p)).collect::<Result<Vec<_>, _>>()?;
    let before: u64 = srcs.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    let bytes = pdf_from_pages(&pages);
    let path = dst("", "pdf");
    std::fs::write(&path, &bytes).map_err(|e| format!("Write failed: {e}"))?;
    Ok(Converted { path, before, after: bytes.len() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-noise: incompressible enough to need real work.
    fn noisy(w: u32, h: u32) -> DynamicImage {
        let mut x = 0x2545F491u32;
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |_, _| {
            let mut n = || {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            };
            image::Rgb([n(), n(), n()])
        }))
    }

    #[test]
    fn fits_under_the_limit_by_quality_then_scale() {
        let img = noisy(600, 600);
        let big = jpeg(&flatten(&img), START_QUALITY).len() as u64;
        // Reachable by quality alone.
        let b = fit_jpeg(&img, big * 3 / 4).expect("quality pass");
        assert!(b.len() as u64 <= big * 3 / 4);
        // Needs shrinking: far below what MIN_QUALITY can reach at full size.
        let tiny = 20_000;
        let b = fit_jpeg(&img, tiny).expect("scale pass");
        assert!(b.len() as u64 <= tiny);
        let d = image::load_from_memory(&b).unwrap();
        assert!(d.width() < 600, "image was scaled down");
    }

    #[test]
    fn compress_skips_small_files_and_never_overwrites() {
        let dir = std::env::temp_dir().join("hytte-convert");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("small.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).save(&src).unwrap();
        let err = compress(&src, 5, |_, _| unreachable!()).err().unwrap();
        assert!(err.starts_with("Already under"), "{err}");
    }

    #[test]
    fn transparent_pixels_flatten_to_white() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0])));
        assert_eq!(flatten(&img).get_pixel(0, 0).0, [255, 255, 255]);
    }

    #[test]
    fn pdf_structure_and_xref_offsets_resolve() {
        let page = |w, h| PdfPage { w, h, gray: false, dct: true, data: vec![1, 2, 3] };
        let pdf = pdf_from_pages(&[page(100, 200), page(300, 100)]);
        // Offsets are byte offsets; the header has non-UTF-8 bytes, so only slice the ASCII tail as text.
        assert!(pdf.starts_with(b"%PDF-") && pdf.ends_with(b"%%EOF
"));
        let find = |needle: &[u8]| pdf.windows(needle.len()).rposition(|w| w == needle).unwrap();
        let tail = String::from_utf8_lossy(&pdf[find(b"xref
0 ")..]).into_owned();
        let sx: usize = tail.rsplit("startxref
").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert_eq!(sx, find(b"xref
0 "), "startxref points at the table");
        assert!(tail.starts_with("xref
0 9
"), "1 catalog + 1 pages + 2x3");
        for (i, line) in tail.lines().skip(3).take(8).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
        }
        assert!(pdf.windows(8).any(|w| w == b"/Count 2"));
    }

    #[test]
    fn pages_are_fitted_inside_the_margins() {
        let (pg, (x, y, w, h)) = place(4000, 1000); // landscape
        assert_eq!(pg, (842.0, 595.0));
        assert!(x >= MARGIN - 1e-9 && y >= 0.0 && x + w <= pg.0 - MARGIN + 1e-9 && y + h <= pg.1 - MARGIN + 1e-9);
    }

    #[test]
    fn png_to_pdf_roundtrip() {
        let dir = std::env::temp_dir().join("hytte-convert-pdf");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("a.png");
        noisy(64, 64).save(&src).unwrap();
        let out = dir.join("a.pdf");
        let c = to_pdf(&[src], |_, _| out.clone()).unwrap();
        let bytes = std::fs::read(&c.path).unwrap();
        assert!(bytes.starts_with(b"%PDF-") && c.after == bytes.len() as u64);
    }
}
