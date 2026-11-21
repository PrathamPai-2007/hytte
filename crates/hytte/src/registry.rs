//! The pack registry behind `hytte pack`: an `index.json` listing packs, each file with its size
//! and SHA-256, next to the pack folders (`examples/panes/` in the Hytte repository by default).
//!
//! Trust model, in order: the index and every file come over HTTPS from the source you chose;
//! every file must match the size and SHA-256 the index lists, or nothing is written; and what
//! arrives is still an *unanswered* pack, so the pill shows its command with Run and Ignore
//! before anything runs. An install or update forgets any earlier answer for that folder, because
//! an answer covers `pane.toml` only and an update may change a script beside it.
//! Nothing here runs a pack's code.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where packs come from unless `--source` says otherwise.
pub const DEFAULT_SOURCE: &str =
    "https://raw.githubusercontent.com/PrathamPai-2007/hytte/main/examples/panes";
pub const MARKER: &str = ".hytte-pack.json";
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_FILES: usize = 20;
const MAX_INDEX_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    /// The oldest Hytte that can run it.
    #[serde(default)]
    pub min_hytte: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Deserialize)]
struct Index {
    version: u32,
    packs: Vec<Entry>,
}

/// Kept in an installed folder, so `update` and `remove` know what is theirs.
#[derive(Debug, Serialize, Deserialize)]
struct Installed {
    name: String,
    version: String,
    source: String,
    files: Vec<String>,
}

/// Where the bytes come from: a web address or a folder (for your own collection or offline).
pub trait Source {
    fn get(&self, rel: &str, max: u64) -> Result<Vec<u8>, String>;
    fn label(&self) -> String;
}

pub struct Remote(pub String);
pub struct Local(pub PathBuf);

impl Source for Remote {
    fn get(&self, rel: &str, max: u64) -> Result<Vec<u8>, String> {
        let url = format!("{}/{rel}", self.0.trim_end_matches('/'));
        crate::net::fetch(&url, None, 30, max)
    }
    fn label(&self) -> String {
        self.0.clone()
    }
}

impl Source for Local {
    fn get(&self, rel: &str, max: u64) -> Result<Vec<u8>, String> {
        let p = self.0.join(rel);
        let b = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if b.len() as u64 > max {
            return Err(format!("{rel} is bigger than {max} bytes"));
        }
        Ok(b)
    }
    fn label(&self) -> String {
        self.0.display().to_string()
    }
}

/// `https://...` is a web source; anything else is a folder. None is the default.
pub fn source_from(arg: Option<&str>) -> Box<dyn Source> {
    match arg {
        None => Box::new(Remote(DEFAULT_SOURCE.into())),
        Some(s) if s.starts_with("https://") || s.starts_with("http://") => {
            Box::new(Remote(s.into()))
        }
        Some(s) => Box::new(Local(PathBuf::from(s))),
    }
}

// ---- SHA-256 (FIPS 180-4), so no hashing crate is linked for one use ----

pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

// ---- the index ----

/// A file name inside a pack: plain, flat, no way out of the folder.
fn safe_file(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn valid_entry(e: &Entry) -> Result<(), String> {
    let bad = |why: &str| Err(format!("index: \"{}\": {why}", e.name));
    if e.name.is_empty() || e.name.len() > 20 || e.name != crate::packs::prefix(&e.name) {
        return Err(format!(
            "index: \"{}\": a name is lower-case letters, digits, - and _ (20 at most)",
            e.name
        ));
    }
    if e.version.is_empty() || e.version.len() > 20 {
        return bad("no version");
    }
    if !e.min_hytte.is_empty() && crate::update::parse(&e.min_hytte).is_none() {
        return bad("min_hytte is not a version");
    }
    if e.files.is_empty() || e.files.len() > MAX_FILES {
        return bad("1 to 20 files");
    }
    if !e.files.iter().any(|f| f.path == "pane.toml") {
        return bad("no pane.toml");
    }
    for f in &e.files {
        if !safe_file(&f.path) {
            return bad("a file name is not plain");
        }
        if f.size == 0 || f.size > MAX_FILE_BYTES {
            return bad("a file is empty or over 64 KB");
        }
        if f.sha256.len() != 64 || !f.sha256.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            return bad("a sha256 is not 64 lower-case hex digits");
        }
    }
    let mut paths: Vec<_> = e
        .files
        .iter()
        .map(|f| f.path.to_ascii_lowercase())
        .collect();
    paths.sort();
    paths.dedup();
    if paths.len() != e.files.len() {
        return bad("a file is listed twice");
    }
    Ok(())
}

pub fn parse_index(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    // A file saved by Windows PowerShell 5 starts with a byte-order mark.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let ix: Index = serde_json::from_slice(bytes).map_err(|e| format!("index.json: {e}"))?;
    if ix.version != 1 {
        return Err(format!(
            "index.json is version {}; this Hytte reads version 1 (update Hytte)",
            ix.version
        ));
    }
    let mut seen: Vec<&str> = vec![];
    for e in &ix.packs {
        valid_entry(e)?;
        if seen.contains(&e.name.as_str()) {
            return Err(format!("index: \"{}\" is listed twice", e.name));
        }
        seen.push(&e.name);
    }
    Ok(ix.packs)
}

pub fn load_index(src: &dyn Source) -> Result<Vec<Entry>, String> {
    parse_index(&src.get("index.json", MAX_INDEX_BYTES)?)
}

fn find<'a>(index: &'a [Entry], name: &str) -> Result<&'a Entry, String> {
    index
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| format!("no pack called \"{name}\" (try `hytte pack search`)"))
}

pub fn search(index: &[Entry], query: &str) -> String {
    let q = query.trim().to_lowercase();
    let hits: Vec<_> = index
        .iter()
        .filter(|e| {
            q.is_empty()
                || e.name.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
        })
        .collect();
    if hits.is_empty() {
        return "nothing found".into();
    }
    hits.iter()
        .map(|e| format!("{:<14} {:<8} {}", e.name, e.version, e.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every file, downloaded and checked against the index: size first, then SHA-256.
fn fetch_files(src: &dyn Source, e: &Entry) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = vec![];
    for f in &e.files {
        let b = src.get(&format!("{}/{}", e.name, f.path), MAX_FILE_BYTES)?;
        if b.len() as u64 != f.size || sha256_hex(&b) != f.sha256 {
            return Err(format!(
                "{}/{} does not match the index (size or SHA-256): nothing was installed",
                e.name, f.path
            ));
        }
        out.push((f.path.clone(), b));
    }
    Ok(out)
}

fn too_new(e: &Entry, current: &str) -> Result<(), String> {
    if e.min_hytte.is_empty() {
        return Ok(());
    }
    match (
        crate::update::parse(&e.min_hytte),
        crate::update::parse(current),
    ) {
        (Some(need), Some(have)) if need > have => Err(format!(
            "{} needs Hytte {} or newer; this is {current}",
            e.name, e.min_hytte
        )),
        _ => Ok(()),
    }
}

/// What the pack would run, from its verified `pane.toml`: the lines a person should read.
pub fn info(src: &dyn Source, index: &[Entry], name: &str) -> Result<String, String> {
    let e = find(index, name)?;
    let files = fetch_files(src, e)?;
    let toml = files
        .iter()
        .find(|(p, _)| p == "pane.toml")
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    let p = crate::packs::parse(&e.name, Path::new(""), &toml, None);
    let mut s = format!("{} {}  {}\n", e.name, e.version, e.description);
    if let crate::packs::State::Bad(why) = &p.state {
        s.push_str(&format!("its pane.toml has a mistake: {why}\n"));
    } else {
        s.push_str(&format!(
            "pane \"{}\" runs, every {} s while open:\n  {}\n",
            p.name, p.every, p.command
        ));
        for a in &p.actions {
            s.push_str(&format!(
                "action {}{}:\n  {}\n",
                a.name,
                if a.confirm { " (asks Sure?)" } else { "" },
                a.command
            ));
        }
    }
    s.push_str("files: ");
    s.push_str(
        &e.files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    );
    s.push_str("\nRead the scripts before you approve: your answer covers pane.toml, not them.");
    Ok(s)
}

fn read_marker(dir: &Path) -> Option<Installed> {
    serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()
}

fn write_files(dir: &Path, files: &[(String, Vec<u8>)], m: &Installed) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    for (p, b) in files {
        std::fs::write(dir.join(p), b).map_err(|e| format!("couldn't write {p}: {e}"))?;
    }
    let text = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(MARKER), text).map_err(|e| e.to_string())
}

const NEXT: &str = "Hytte shows a card with its command: press Run to turn it on. Your answer covers pane.toml only, so read the scripts first.";

/// `forget(folder)` drops any earlier Run/Ignore answer, so the pack asks again.
pub fn install(
    src: &dyn Source,
    index: &[Entry],
    packs: &Path,
    name: &str,
    current: &str,
    forget: &dyn Fn(&str),
) -> Result<String, String> {
    let e = find(index, name)?;
    too_new(e, current)?;
    let dir = packs.join(&e.name);
    if dir.exists() {
        return Err(match read_marker(&dir) {
            Some(m) => format!(
                "{} {} is already installed (try `hytte pack update {}`)",
                e.name, m.version, e.name
            ),
            None => format!(
                "the folder {} already exists and was not installed by `hytte pack`; it is left alone",
                dir.display()
            ),
        });
    }
    let files = fetch_files(src, e)?;
    let m = Installed {
        name: e.name.clone(),
        version: e.version.clone(),
        source: src.label(),
        files: e.files.iter().map(|f| f.path.clone()).collect(),
    };
    if let Err(err) = write_files(&dir, &files, &m) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err);
    }
    forget(&e.name);
    Ok(format!("Installed {} {}. {NEXT}", e.name, e.version))
}

/// One installed pack (or all of them when `name` is None) to the index's version. Files of
/// yours beside the pack (a `repo.txt` you made) are never touched.
pub fn update(
    src: &dyn Source,
    index: &[Entry],
    packs: &Path,
    name: Option<&str>,
    current: &str,
    forget: &dyn Fn(&str),
) -> Result<String, String> {
    let names: Vec<String> = match name {
        Some(n) => vec![n.trim().to_string()],
        None => installed(packs).into_iter().map(|(n, _)| n).collect(),
    };
    if names.is_empty() {
        return Ok("no packs installed with `hytte pack install`".into());
    }
    let mut lines = vec![];
    for n in names {
        let dir = packs.join(&n);
        let Some(old) = read_marker(&dir) else {
            lines.push(format!("{n}: not installed by `hytte pack`, left alone"));
            continue;
        };
        let e = match find(index, &n) {
            Ok(e) => e,
            Err(_) => {
                lines.push(format!("{n}: no longer in the index, left as it is"));
                continue;
            }
        };
        if e.version == old.version {
            lines.push(format!("{n}: up to date ({})", old.version));
            continue;
        }
        if let Err(err) = too_new(e, current) {
            lines.push(format!("{n}: {err}"));
            continue;
        }
        let files = match fetch_files(src, e) {
            Ok(f) => f,
            Err(err) => {
                lines.push(format!("{n}: {err}"));
                continue;
            }
        };
        let m = Installed {
            name: e.name.clone(),
            version: e.version.clone(),
            source: src.label(),
            files: e.files.iter().map(|f| f.path.clone()).collect(),
        };
        if let Err(err) = write_files(&dir, &files, &m) {
            lines.push(format!("{n}: {err}"));
            continue;
        }
        // A file the new version no longer has goes; anything else in the folder stays.
        for gone in old.files.iter().filter(|f| !m.files.contains(f)) {
            if safe_file(gone) {
                let _ = std::fs::remove_file(dir.join(gone));
            }
        }
        forget(&e.name);
        lines.push(format!(
            "{n}: updated {} -> {}. {NEXT}",
            old.version, e.version
        ));
    }
    Ok(lines.join("\n"))
}

/// Only a pack `hytte pack` installed, and only its own files; yours stay.
pub fn remove(packs: &Path, name: &str, forget: &dyn Fn(&str)) -> Result<String, String> {
    let n = name.trim();
    if n != crate::packs::prefix(n) || n.is_empty() {
        return Err(format!("\"{n}\" is not a pack name"));
    }
    let dir = packs.join(n);
    let Some(m) = read_marker(&dir) else {
        return Err(format!(
            "\"{n}\" was not installed by `hytte pack`: delete its folder yourself if you want it gone"
        ));
    };
    for f in m.files.iter().filter(|f| safe_file(f)) {
        let _ = std::fs::remove_file(dir.join(f));
    }
    let _ = std::fs::remove_file(dir.join(MARKER));
    forget(n);
    match std::fs::remove_dir(&dir) {
        Ok(()) => Ok(format!("Removed {n}.")),
        Err(_) => Ok(format!(
            "Removed {n}'s own files; your other files in {} were kept.",
            dir.display()
        )),
    }
}

/// (name, version) of every pack `hytte pack` installed.
pub fn installed(packs: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(packs)
        .map(|d| {
            d.flatten()
                .filter_map(|e| read_marker(&e.path()).map(|m| (m.name, m.version)))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Two blocks: the padding crosses a block boundary.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    /// A source folder with one pack, and its index entry.
    struct Fixture {
        root: PathBuf,
        packs: PathBuf,
        entry: Entry,
    }

    fn fixture(tag: &str, files: &[(&str, &str)]) -> Fixture {
        let root = std::env::temp_dir().join(format!("hytte-reg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("src").join("demo");
        std::fs::create_dir_all(&pack).unwrap();
        let mut fe = vec![];
        for (p, text) in files {
            std::fs::write(pack.join(p), text).unwrap();
            fe.push(FileEntry {
                path: (*p).into(),
                sha256: sha256_hex(text.as_bytes()),
                size: text.len() as u64,
            });
        }
        let packs = root.join("packs");
        std::fs::create_dir_all(&packs).unwrap();
        Fixture {
            root,
            packs,
            entry: Entry {
                name: "demo".into(),
                description: "a demo".into(),
                version: "1.0.0".into(),
                min_hytte: "1.9.0".into(),
                files: fe,
            },
        }
    }

    const TOML: &str = "name = \"Demo\"\ncommand = \"powershell -File d.ps1\"\nevery = 60\n";
    fn src(f: &Fixture) -> Local {
        Local(f.root.join("src"))
    }
    fn files() -> [(&'static str, &'static str); 2] {
        [("pane.toml", TOML), ("d.ps1", "'hello'\n")]
    }

    #[test]
    fn install_verifies_writes_and_leaves_the_pack_unanswered() {
        let f = fixture("inst", &files());
        let forgot = RefCell::new(vec![]);
        let forget = |n: &str| forgot.borrow_mut().push(n.to_string());
        let msg = install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "DEMO",
            "1.9.3",
            &forget,
        )
        .unwrap();
        assert!(msg.contains("press Run"), "{msg}");
        assert!(f.packs.join("demo").join("d.ps1").exists());
        assert_eq!(
            forgot.borrow().as_slice(),
            ["demo"],
            "an old answer is dropped"
        );
        // What arrives is a pack waiting for a human, not an approved one.
        let text = std::fs::read_to_string(f.packs.join("demo").join("pane.toml")).unwrap();
        let p = crate::packs::parse("demo", &f.packs.join("demo"), &text, None);
        assert_eq!(p.state, crate::packs::State::Pending);
        assert_eq!(
            installed(&f.packs),
            [("demo".to_string(), "1.0.0".to_string())]
        );
        // Twice: told to update instead.
        let again = install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "demo",
            "1.9.3",
            &forget,
        );
        assert!(again.unwrap_err().contains("already installed"));
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn a_tampered_file_installs_nothing() {
        let f = fixture("tamper", &files());
        std::fs::write(f.root.join("src").join("demo").join("d.ps1"), "'evil'\n").unwrap();
        let e = install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "demo",
            "1.9.3",
            &|_| {},
        )
        .unwrap_err();
        assert!(e.contains("does not match the index"), "{e}");
        assert!(!f.packs.join("demo").exists(), "not even a partial folder");
        // The same size with different bytes is caught by the hash, not just the size.
        std::fs::write(f.root.join("src").join("demo").join("d.ps1"), "'hellp'\n").unwrap();
        assert!(install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "demo",
            "1.9.3",
            &|_| {}
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn it_refuses_a_folder_it_did_not_make_and_a_hytte_that_is_too_old() {
        let f = fixture("refuse", &files());
        std::fs::create_dir_all(f.packs.join("demo")).unwrap();
        std::fs::write(f.packs.join("demo").join("pane.toml"), "mine").unwrap();
        let e = install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "demo",
            "1.9.3",
            &|_| {},
        )
        .unwrap_err();
        assert!(e.contains("not installed by"), "{e}");
        assert_eq!(
            std::fs::read_to_string(f.packs.join("demo").join("pane.toml")).unwrap(),
            "mine"
        );
        let mut old = f.entry.clone();
        old.min_hytte = "9.0.0".into();
        let _ = std::fs::remove_dir_all(f.packs.join("demo"));
        let e = install(&src(&f), &[old], &f.packs, "demo", "1.9.3", &|_| {}).unwrap_err();
        assert!(e.contains("needs Hytte 9.0.0"), "{e}");
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn an_update_keeps_your_files_drops_removed_ones_and_asks_again() {
        let f = fixture("upd", &files());
        install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "demo",
            "1.9.3",
            &|_| {},
        )
        .unwrap();
        std::fs::write(f.packs.join("demo").join("repo.txt"), "mine").unwrap();
        // Version 1.1.0: d.ps1 is replaced by e.ps1.
        let g = fixture("upd2", &[("pane.toml", TOML), ("e.ps1", "'two'\n")]);
        let mut e2 = g.entry.clone();
        e2.version = "1.1.0".into();
        let forgot = RefCell::new(0);
        let forget = |_: &str| *forgot.borrow_mut() += 1;
        let msg = update(&src(&g), &[e2.clone()], &f.packs, None, "1.9.3", &forget).unwrap();
        assert!(msg.contains("1.0.0 -> 1.1.0"), "{msg}");
        let d = f.packs.join("demo");
        assert!(d.join("e.ps1").exists() && !d.join("d.ps1").exists());
        assert_eq!(std::fs::read_to_string(d.join("repo.txt")).unwrap(), "mine");
        assert_eq!(*forgot.borrow(), 1);
        // Same version again: nothing to do, nothing forgotten.
        let msg = update(&src(&g), &[e2], &f.packs, None, "1.9.3", &forget).unwrap();
        assert!(msg.contains("up to date"), "{msg}");
        assert_eq!(*forgot.borrow(), 1);
        let _ = std::fs::remove_dir_all(&f.root);
        let _ = std::fs::remove_dir_all(&g.root);
    }

    #[test]
    fn remove_takes_only_what_the_pack_brought() {
        let f = fixture("rm", &files());
        install(
            &src(&f),
            &[f.entry.clone()],
            &f.packs,
            "demo",
            "1.9.3",
            &|_| {},
        )
        .unwrap();
        std::fs::write(f.packs.join("demo").join("repo.txt"), "mine").unwrap();
        let msg = remove(&f.packs, "demo", &|_| {}).unwrap();
        assert!(msg.contains("your other files"), "{msg}");
        let d = f.packs.join("demo");
        assert!(d.join("repo.txt").exists() && !d.join("pane.toml").exists());
        // A pack it didn't install, or a name that climbs out: refused.
        std::fs::create_dir_all(f.packs.join("hand")).unwrap();
        assert!(remove(&f.packs, "hand", &|_| {}).is_err());
        assert!(remove(&f.packs, "..", &|_| {}).is_err());
        assert!(remove(&f.packs, r"..\x", &|_| {}).is_err());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn the_index_rejects_anything_that_could_escape_or_surprise() {
        let good = |files: &str| {
            format!(
                r#"{{"version":1,"packs":[{{"name":"demo","description":"d","version":"1.0.0","min_hytte":"1.9.0","files":{files}}}]}}"#
            )
        };
        let h = "0".repeat(64);
        let ok = format!(r#"[{{"path":"pane.toml","sha256":"{h}","size":10}}]"#);
        assert!(parse_index(good(&ok).as_bytes()).is_ok());
        for path in ["..\\\\x", "a/b", ".hidden", "..", "", "con:x", "sp ace"] {
            let f = format!(
                r#"[{{"path":"pane.toml","sha256":"{h}","size":10}},{{"path":"{path}","sha256":"{h}","size":10}}]"#
            );
            assert!(parse_index(good(&f).as_bytes()).is_err(), "{path:?}");
        }
        for bad in [
            format!(r#"[{{"path":"x.ps1","sha256":"{h}","size":10}}]"#),
            format!(r#"[{{"path":"pane.toml","sha256":"{h}","size":0}}]"#),
            format!(r#"[{{"path":"pane.toml","sha256":"{h}","size":99999999}}]"#),
            r#"[{"path":"pane.toml","sha256":"ABC","size":10}]"#.to_string(),
            "[]".to_string(),
        ] {
            assert!(parse_index(good(&bad).as_bytes()).is_err(), "{bad}");
        }
        assert!(parse_index(br#"{"version":2,"packs":[]}"#).is_err());
        assert!(parse_index(b"not json").is_err());
        let dup = format!(
            r#"{{"version":1,"packs":[{{"name":"a","version":"1","files":{ok}}},{{"name":"a","version":"1","files":{ok}}}]}}"#
        );
        assert!(parse_index(dup.as_bytes()).is_err());
        let bad_name = good(&ok).replace("\"demo\"", "\"Demo Pack\"");
        assert!(parse_index(bad_name.as_bytes()).is_err());
    }

    #[test]
    fn info_shows_what_would_run_and_search_finds_by_word() {
        let f = fixture("info", &files());
        let s = info(&src(&f), &[f.entry.clone()], "demo").unwrap();
        assert!(
            s.contains("powershell -File d.ps1") && s.contains("Read the scripts"),
            "{s}"
        );
        assert!(search(&[f.entry.clone()], "DEM").contains("demo"));
        assert!(search(&[f.entry.clone()], "zzz").contains("nothing found"));
        let _ = std::fs::remove_dir_all(&f.root);
    }
}
