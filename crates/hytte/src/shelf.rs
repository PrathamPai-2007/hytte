//! Staging shelf: files and text snippets held in the pill until dragged out.
//! Items either reference the original path or live as copies under
//! `%APPDATA%\Hytte\shelf`. The list persists to `shelf.json`.

use crate::config::{data_dir, Shelf};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShelfItem {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// Lives in our own shelf folder (so removing the item deletes the copy).
    #[serde(default)]
    pub owned: bool,
}

fn store_dir() -> PathBuf {
    data_dir().join("shelf")
}

fn list_path() -> PathBuf {
    data_dir().join("shelf.json")
}

fn next_id(items: &[ShelfItem]) -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    t.max(items.iter().map(|i| i.id + 1).max().unwrap_or(1))
}

/// Load the persisted shelf, dropping entries whose file vanished.
pub fn load() -> Vec<ShelfItem> {
    let text = std::fs::read_to_string(list_path()).unwrap_or_default();
    let mut items: Vec<ShelfItem> = serde_json::from_str(&text).unwrap_or_default();
    items.retain(|i| i.path.exists());
    items
}

pub fn save(items: &[ShelfItem]) {
    if let Some(p) = list_path().parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Ok(t) = serde_json::to_string_pretty(items) {
        let _ = std::fs::write(list_path(), t);
    }
}

fn unique_in(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("item");
    let ext = p.extension().and_then(|s| s.to_str());
    let mut i = 0;
    loop {
        let n = match (i, ext) {
            (0, Some(e)) => format!("{stem}.{e}"),
            (0, None) => stem.to_string(),
            (_, Some(e)) => format!("{stem}-{i}.{e}"),
            (_, None) => format!("{stem}-{i}"),
        };
        let c = dir.join(n);
        if !c.exists() {
            return c;
        }
        i += 1;
    }
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// File work for a drop, safe to run off the UI thread: in copy mode the paths
/// are copied into the shelf folder, and dropped text is written as a snippet.
/// Returns items without ids, in drop order; at most `room` of them, skipping
/// missing paths and (reference mode) paths already in `existing`. Hand the
/// result to [`insert`] on the thread that owns the shelf.
pub fn stage(
    cfg: &Shelf,
    paths: &[PathBuf],
    text: Option<&str>,
    existing: &[PathBuf],
    room: usize,
) -> Vec<ShelfItem> {
    // Serialises picking a free name and creating it, so two drops copying
    // files with the same name at once cannot both pick `name.ext`.
    static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let mut out: Vec<ShelfItem> = vec![];
    for p in paths {
        if out.len() >= room
            || !p.exists()
            || existing.contains(p)
            || out.iter().any(|i| !i.owned && &i.path == p)
        {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("item")
            .to_string();
        let (path, owned) = if cfg.mode == "copy" {
            let _store = STORE.lock().unwrap_or_else(|e| e.into_inner());
            let _ = std::fs::create_dir_all(store_dir());
            let dst = unique_in(&store_dir(), &name);
            if copy_recursive(p, &dst).is_err() {
                // Don't leave a half-copied folder behind.
                let _ = if dst.is_dir() {
                    std::fs::remove_dir_all(&dst)
                } else {
                    std::fs::remove_file(&dst)
                };
                continue;
            }
            (dst, true)
        } else {
            (p.clone(), false)
        };
        out.push(ShelfItem {
            id: 0,
            is_dir: path.is_dir(),
            path,
            name,
            owned,
        });
    }
    if let Some(text) = text.filter(|t| out.len() < room && !t.trim().is_empty()) {
        let _store = STORE.lock().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::create_dir_all(store_dir());
        let first: String = text
            .trim()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ')
            .take(24)
            .collect();
        let name = format!(
            "{}.txt",
            if first.trim().is_empty() {
                "snippet"
            } else {
                first.trim()
            }
        );
        let path = unique_in(&store_dir(), &name);
        if std::fs::write(&path, text).is_ok() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("snippet.txt")
                .to_string();
            out.push(ShelfItem {
                id: 0,
                path,
                name,
                is_dir: false,
                owned: true,
            });
        }
    }
    out
}

/// Add staged items, assigning ids. The shelf may have changed while they were
/// staged, so limits and duplicates are checked again; a copy that no longer
/// fits is deleted. Returns how many were added.
pub fn insert(cfg: &Shelf, items: &mut Vec<ShelfItem>, staged: Vec<ShelfItem>) -> usize {
    let mut n = 0;
    for mut it in staged {
        if items.len() >= cfg.max_items || (!it.owned && items.iter().any(|i| i.path == it.path)) {
            if it.owned {
                let _ = if it.is_dir {
                    std::fs::remove_dir_all(&it.path)
                } else {
                    std::fs::remove_file(&it.path)
                };
            }
            continue;
        }
        it.id = next_id(items);
        items.push(it);
        n += 1;
    }
    n
}

/// Free slots on the shelf.
pub fn room(cfg: &Shelf, items: &[ShelfItem]) -> usize {
    cfg.max_items.saturating_sub(items.len())
}

/// Paths already on the shelf (for [`stage`]'s duplicate check).
pub fn paths(items: &[ShelfItem]) -> Vec<PathBuf> {
    items.iter().map(|i| i.path.clone()).collect()
}

/// Stage and insert in one go (blocking); returns how many were added.
#[cfg(test)]
pub fn add_paths(cfg: &Shelf, items: &mut Vec<ShelfItem>, paths_in: &[PathBuf]) -> usize {
    let staged = stage(cfg, paths_in, None, &paths(items), room(cfg, items));
    insert(cfg, items, staged)
}

/// Stash dropped text as a snippet file so it can be dragged out like any file.
#[cfg(test)]
pub fn add_text(cfg: &Shelf, items: &mut Vec<ShelfItem>, text: &str) -> usize {
    let staged = stage(cfg, &[], Some(text), &[], room(cfg, items));
    insert(cfg, items, staged)
}

/// Remove an item; deletes the file only if the shelf owns it.
pub fn remove(items: &mut Vec<ShelfItem>, id: u64) {
    if let Some(i) = items.iter().position(|i| i.id == id) {
        let it = items.remove(i);
        if it.owned {
            let _ = if it.is_dir {
                std::fs::remove_dir_all(&it.path)
            } else {
                std::fs::remove_file(&it.path)
            };
        }
    }
}

/// Shell thumbnail (or file-type icon) as a 64x64 premultiplied BGRA tile.
#[cfg(windows)]
pub fn thumbnail(path: &Path) -> Option<crate::ui_state::ArtBitmap> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT,
    };
    unsafe {
        let f: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let hbm = f
            .GetImage(SIZE { cx: 64, cy: 64 }, SIIGBF_RESIZETOFIT)
            .ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(
            hbm.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        let (w, h) = (bm.bmWidth.max(1), bm.bmHeight.max(1));
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let got = GetDIBits(
            dc,
            hbm,
            0,
            h as u32,
            Some(px.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        let _ = DeleteObject(hbm.into());
        if got == 0 {
            return None;
        }
        // Shell bitmaps are premultiplied ARGB; some carry no alpha at all.
        if px.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            px.as_chunks_mut::<4>()
                .0
                .iter_mut()
                .for_each(|p| p[3] = 255);
        }
        // Centre inside a 64x64 tile.
        let mut out = vec![0u8; 64 * 64 * 4];
        let (ox, oy) = ((64 - w.min(64)) / 2, (64 - h.min(64)) / 2);
        for y in 0..h.min(64) {
            for x in 0..w.min(64) {
                let s = ((y * w + x) * 4) as usize;
                let d = (((y + oy) * 64 + x + ox) * 4) as usize;
                out[d..d + 4].copy_from_slice(&px[s..s + 4]);
            }
        }
        Some(crate::ui_state::ArtBitmap {
            w: 64,
            h: 64,
            bgra: out,
        })
    }
}

/// Remove an item from the list without touching its file (it was just dragged out).
pub fn remove_keep_file(items: &mut Vec<ShelfItem>, id: u64) {
    items.retain(|i| i.id != id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_dedup_copy_and_remove() {
        let dir = std::env::temp_dir().join("hytte-shelf-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "hi").unwrap();
        let mut cfg = Shelf::default();
        let mut items = vec![];
        assert_eq!(add_paths(&cfg, &mut items, &[f.clone(), f.clone()]), 1);
        assert_eq!(add_paths(&cfg, &mut items, std::slice::from_ref(&f)), 0);
        assert_eq!(items[0].path, f);
        let id0 = items[0].id;
        remove(&mut items, id0);
        assert!(f.exists(), "reference items never delete the original");

        cfg.max_items = 1;
        assert_eq!(add_paths(&cfg, &mut vec![], &[dir.join("missing")]), 0);
        let mut items = vec![];
        assert_eq!(add_text(&Shelf::default(), &mut items, "hello world"), 1);
        assert!(items[0].path.exists() && items[0].owned);
        let p = items[0].path.clone();
        let id0 = items[0].id;
        remove(&mut items, id0);
        assert!(!p.exists());
    }

    #[test]
    fn copy_mode_stages_then_inserts_within_limits() {
        let dir = std::env::temp_dir().join("hytte-shelf-copy-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let f = dir.join("b.txt");
        std::fs::write(&f, "hi").unwrap();
        std::fs::write(dir.join("folder").join("c.txt"), "x").unwrap();
        let cfg = Shelf {
            mode: "copy".into(),
            max_items: 2,
            ..Shelf::default()
        };
        let staged = stage(&cfg, &[f.clone(), dir.join("folder")], None, &[], 2);
        assert_eq!(staged.len(), 2);
        assert!(staged
            .iter()
            .all(|i| i.owned && i.path.exists() && i.path != f));
        assert!(staged[1].is_dir && staged[1].path.join("c.txt").exists());
        // The shelf filled up while copying: the copy that no longer fits is deleted.
        let mut items = vec![];
        add_text(&cfg, &mut items, "already here");
        let late = staged[1].path.clone();
        assert_eq!(insert(&cfg, &mut items, staged), 1);
        assert_eq!(items.len(), 2);
        assert!(!late.exists());
        for it in items.clone() {
            remove(&mut items, it.id);
        }
        assert!(f.exists(), "the original is untouched");
    }
}
