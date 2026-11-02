//! Single TOML settings file in %APPDATA%\Hytte.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub autostart: bool,
    /// Put the folder holding `hytte.exe` on the user's PATH at startup.
    pub add_to_path: bool,
    /// Which monitor the pill hangs from: "primary", "active" (the foreground window's) or
    /// "cursor" (under the pointer; re-checked when the foreground window changes).
    pub monitor: String,
    /// Tint the media glow with the album art's dominant colour.
    pub adaptive_glow: bool,
    /// How strong the border glow is: 1.0 is the original look, 0 turns it off.
    pub glow_strength: f32,
    /// Global hotkey that opens the pill for keyboard use, e.g. "Win+Alt+N". Empty = off.
    pub hotkey: String,
    /// "dark" (default), "light", or "auto" (follow the Windows app theme).
    pub theme: String,
    /// Over a maximised window or a fullscreen video: "line" (shrink to a hairline), "hide",
    /// or "off" (stay as usual). Games always hide it completely.
    pub fullscreen: String,
    pub output_folder: Option<String>,
    pub allow_list: Vec<String>,
    pub deny_list: Vec<String>,
    /// Seconds the pill opens to show a finished or failed task (0 = don't open).
    pub finish_peek_secs: u64,
    /// Keep the pill out of screenshots, recordings and screen share.
    pub hide_from_capture: bool,
    /// Look for a newer release on GitHub once a day and offer to install it. Read at startup.
    pub check_updates: bool,
}

/// Modifier bits as `RegisterHotKey` wants them.
pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

/// Parse a hotkey such as "Win+Alt+N" or "ctrl+shift+F9" into (modifiers, virtual key).
/// Needs at least one modifier and exactly one key (a letter, a digit or F1-F24), so a
/// typo can't claim a bare key system-wide.
pub fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    let (mut mods, mut key) = (0u32, None);
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let p = part.to_ascii_lowercase();
        match p.as_str() {
            "alt" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "win" | "windows" | "super" => mods |= MOD_WIN,
            _ => {
                let vk = match p.as_bytes() {
                    [c @ b'a'..=b'z'] => Some(c.to_ascii_uppercase() as u32),
                    [c @ b'0'..=b'9'] => Some(*c as u32),
                    [b'f', rest @ ..] => std::str::from_utf8(rest)
                        .ok()
                        .and_then(|n| n.parse::<u32>().ok())
                        .filter(|n| (1..=24).contains(n))
                        .map(|n| 0x70 + n - 1),
                    _ => None,
                };
                if key.replace(vk?).is_some() {
                    return None;
                }
            }
        }
    }
    (mods != 0).then_some(())?;
    Some((mods, key?))
}

impl Default for General {
    fn default() -> Self {
        Self {
            autostart: false,
            add_to_path: true,
            monitor: "primary".into(),
            adaptive_glow: true,
            glow_strength: 1.2,
            hotkey: String::new(),
            theme: "dark".into(),
            fullscreen: "line".into(),
            output_folder: None,
            allow_list: vec![],
            deny_list: vec![],
            finish_peek_secs: 5,
            hide_from_capture: true,
            check_updates: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shell {
    /// Commands shorter than this never show (the daemon enforces it).
    pub threshold_ms: u32,
    pub ignore: Vec<String>,
}
impl Default for Shell {
    fn default() -> Self {
        Self {
            threshold_ms: 3000,
            ignore: [
                "vim",
                "nvim",
                "vi",
                "nano",
                "less",
                "more",
                "man",
                "ssh",
                "top",
                "htop",
                "tmux",
                "fzf",
                "git-credential-manager",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shelf {
    /// "reference" keeps the original path; "copy" stashes a copy in %APPDATA%\Hytte\shelf.
    pub mode: String,
    pub max_items: usize,
    pub persist: bool,
    pub remove_after_drag: bool,
    /// Put Snipping Tool screenshots (Win+Shift+S) on the shelf as they are copied.
    pub screenshots: bool,
}
impl Default for Shelf {
    fn default() -> Self {
        Self {
            mode: "reference".into(),
            max_items: 12,
            persist: true,
            remove_after_drag: true,
            screenshots: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ports {
    pub watch: Vec<u16>,
    pub show_all: bool,
    pub poll_secs: u64,
}
impl Default for Ports {
    fn default() -> Self {
        Self {
            watch: hytte_proto::ports::DEFAULT_WATCH.to_vec(),
            show_all: false,
            poll_secs: 5,
        }
    }
}

/// A heads-up before the next calendar event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calendar {
    /// Off by default: event titles are private and the pill is on screen. Read at startup.
    pub enabled: bool,
    /// Minutes before the start to show the heads-up.
    pub lead_min: u32,
}
impl Default for Calendar {
    fn default() -> Self {
        Self {
            enabled: false,
            lead_min: 10,
        }
    }
}

/// Show browser downloads as tasks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Downloads {
    pub enabled: bool,
    /// Folder to watch; empty = the user's Downloads folder. Read at startup.
    pub folder: Option<String>,
}
impl Default for Downloads {
    fn default() -> Self {
        Self {
            enabled: true,
            folder: None,
        }
    }
}

/// Alert when one process keeps using a lot of CPU or memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hog {
    pub enabled: bool,
    /// Percent of the whole machine, all cores together.
    pub cpu_pct: u32,
    /// Working set in MB.
    pub mem_mb: u64,
    /// How long the CPU has to stay over the limit.
    pub secs: u64,
}
impl Default for Hog {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_pct: 80,
            mem_mb: 4096,
            secs: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Agent {
    pub peek_secs: u64,
    pub sound: bool,
}
impl Default for Agent {
    fn default() -> Self {
        Self {
            peek_secs: 6,
            sound: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timer {
    pub sound: bool,
    pub break_min: u32,
    pub long_break_min: u32,
    /// Focus sessions before a long break.
    pub rounds: u32,
}
impl Default for Timer {
    fn default() -> Self {
        Self {
            sound: true,
            break_min: 5,
            long_break_min: 15,
            rounds: 4,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub timer: Timer,
    pub general: General,
    pub shell: Shell,
    pub shelf: Shelf,
    pub ports: Ports,
    pub hog: Hog,
    pub downloads: Downloads,
    pub calendar: Calendar,
    pub agent: Agent,
}

pub fn data_dir() -> PathBuf {
    config_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Hytte").join("config.toml")
}

pub fn load() -> Config {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.trim().is_empty() {
        let cfg = Config::default();
        let _ = save(&cfg);
        return cfg;
    }
    parse(&text).unwrap_or_default()
}

/// `None` for invalid TOML, so a half-typed edit never replaces working settings.
pub fn parse(text: &str) -> Option<Config> {
    let mut cfg: Config = toml::from_str(text).ok()?;
    migrate(&mut cfg, text);
    Some(cfg)
}

/// Settings from before 1.2.2: `suppress_fullscreen = false` / `fullscreen_mode = "hide"` are
/// now `fullscreen = "off"` / `"hide"`. Only when the new key isn't set yet.
fn migrate(cfg: &mut Config, text: &str) {
    let Ok(v) = text.parse::<toml::Table>() else {
        return;
    };
    let Some(g) = v.get("general").and_then(|g| g.as_table()) else {
        return;
    };
    if g.contains_key("fullscreen") {
        return;
    }
    if g.get("suppress_fullscreen").and_then(|x| x.as_bool()) == Some(false) {
        cfg.general.fullscreen = "off".into();
    } else if g.get("fullscreen_mode").and_then(|x| x.as_str()) == Some("hide") {
        cfg.general.fullscreen = "hide".into();
    }
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let text = toml::to_string_pretty(cfg).unwrap_or_default();
    write_atomic(&config_path(), &text)
}

/// Temp file + rename, so the config watcher never reads a half-written file.
fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Watches `config.toml` and sends every valid change to the UI. The thread sleeps on a directory
/// change notification, so it costs nothing until a file in the data folder changes; writes to
/// other files there (`shelf.json`, `timer.json`) are filtered out by comparing the text.
pub fn spawn_watcher(ui_tx: crossbeam_channel::Sender<crate::ui_state::UiEvent>) {
    #[cfg(windows)]
    std::thread::spawn(move || unsafe {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::Storage::FileSystem::{
            FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE,
        };
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
        let Ok(h) = FindFirstChangeNotificationW(
            &HSTRING::from(data_dir().as_os_str()),
            false,
            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
        ) else {
            return;
        };
        let mut last = std::fs::read_to_string(config_path()).unwrap_or_default();
        while WaitForSingleObject(h, INFINITE) == WAIT_OBJECT_0 {
            // Editors save in bursts (truncate, write, rename): let them finish.
            std::thread::sleep(std::time::Duration::from_millis(150));
            if FindNextChangeNotification(h).is_err() {
                return;
            }
            let Ok(text) = std::fs::read_to_string(config_path()) else {
                continue;
            };
            if text == last || text.trim().is_empty() {
                continue;
            }
            // A half-typed, invalid file keeps the current settings until it parses again.
            if let Some(cfg) = parse(&text) {
                if ui_tx
                    .send(crate::ui_state::UiEvent::Config(Box::new(cfg)))
                    .is_err()
                {
                    return;
                }
            }
            last = text;
        }
    });
    #[cfg(not(windows))]
    let _ = ui_tx;
}

/// A value the settings pane writes (see [`set`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Val {
    B(bool),
    S(&'static str),
    I(i64),
    F(f64),
}

impl From<Val> for toml_edit::Value {
    fn from(v: Val) -> Self {
        match v {
            Val::B(b) => b.into(),
            Val::S(s) => s.into(),
            Val::I(i) => i.into(),
            Val::F(f) => f.into(),
        }
    }
}

/// Sets one `section.key` in `config.toml` in place, keeping the user's comments and layout,
/// and returns the settings it now holds. `Ok(None)`: the file doesn't parse, so it is left
/// alone (the user is mid-edit; their file wins).
pub fn set(key: &str, v: Val) -> std::io::Result<Option<Config>> {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => toml::to_string_pretty(&Config::default()).unwrap_or_default(),
    };
    let Some(next) = with_value(&text, key, v) else {
        return Ok(None);
    };
    if next != text {
        write_atomic(&path, &next)?;
    }
    Ok(parse(&next))
}

fn with_value(text: &str, key: &str, v: Val) -> Option<String> {
    let (section, name) = key.split_once('.')?;
    let mut doc: toml_edit::DocumentMut = text.parse().ok()?;
    let table = doc
        .entry(section)
        .or_insert_with(toml_edit::table)
        .as_table_like_mut()?;
    match table.get_mut(name).and_then(|i| i.as_value_mut()) {
        // Keep the spacing and any trailing comment on the line.
        Some(old) => {
            let decor = old.decor().clone();
            *old = v.into();
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(name, toml_edit::value(toml_edit::Value::from(v)));
        }
    }
    Some(doc.to_string())
}

/// Opt-in autostart via HKCU\...\Run. No-op off Windows.
pub fn ensure_autostart(enable: bool) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
            HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
        };
        let sub: Vec<u16> = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run\0"
            .encode_utf16()
            .collect();
        let mut hkey = HKEY::default();
        unsafe {
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(sub.as_ptr()),
                None,
                KEY_SET_VALUE | KEY_QUERY_VALUE,
                &mut hkey,
            ) == ERROR_SUCCESS
            {
                if enable {
                    if let Ok(exe) = std::env::current_exe() {
                        let v: Vec<u16> =
                            format!("\"{}\"\0", exe.display()).encode_utf16().collect();
                        let name: Vec<u16> = "Hytte\0".encode_utf16().collect();
                        let bytes =
                            std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
                        let _ = RegSetValueExW(
                            hkey,
                            windows::core::PCWSTR(name.as_ptr()),
                            None,
                            REG_SZ,
                            Some(bytes),
                        );
                    }
                } else {
                    // Only this copy's entry, or one whose exe is gone: a second copy (a dev
                    // build, a portable one) must not switch off the installed one's autostart.
                    let name: Vec<u16> = "Hytte\0".encode_utf16().collect();
                    let mut buf = [0u16; 1024];
                    let mut len = (buf.len() * 2) as u32;
                    let read = RegQueryValueExW(
                        hkey,
                        windows::core::PCWSTR(name.as_ptr()),
                        None,
                        None,
                        Some(buf.as_mut_ptr() as *mut u8),
                        Some(&mut len),
                    ) == ERROR_SUCCESS;
                    let set = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
                    let set = set.trim_end_matches('\0').trim().trim_matches('"');
                    let mine = std::env::current_exe()
                        .is_ok_and(|e| e.display().to_string().eq_ignore_ascii_case(set));
                    if read && (mine || !std::path::Path::new(set).is_file()) {
                        let _ = RegDeleteValueW(hkey, windows::core::PCWSTR(name.as_ptr()));
                    }
                }
                let _ = RegCloseKey(hkey);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enable;
    }
}

/// Start Menu shortcut so Windows search finds Hytte. Rewritten each launch so a moved exe self-heals.
pub fn ensure_start_menu() {
    #[cfg(windows)]
    std::thread::spawn(|| unsafe {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        let (Ok(exe), Ok(appdata)) = (std::env::current_exe(), std::env::var("APPDATA")) else {
            return;
        };
        let dir = std::path::Path::new(&appdata).join(r"Microsoft\Windows\Start Menu\Programs");
        if !dir.is_dir() {
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let res = (|| -> windows::core::Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(exe.as_os_str()))?;
            if let Some(d) = exe.parent() {
                link.SetWorkingDirectory(&HSTRING::from(d.as_os_str()))?;
            }
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(dir.join("Hytte.lnk").as_os_str()), true)
        })();
        if let Err(e) = res {
            crate::logging::line(&format!("start menu shortcut failed: {e}"));
        }
        CoUninitialize();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_edit_keeps_comments_and_other_keys() {
        let text = "# my settings\n[general]\nautostart = false # login\nadaptive_glow = false\n\n[shell]\n# keep me\nthreshold_ms = 5000\n";
        let next = with_value(text, "general.autostart", Val::B(true)).unwrap();
        assert_eq!(next, text.replace("autostart = false", "autostart = true"));
        let cfg = parse(&next).unwrap();
        assert!(cfg.general.autostart && !cfg.general.adaptive_glow);
        assert_eq!(cfg.shell.threshold_ms, 5000);
    }

    #[test]
    fn autostart_edit_adds_missing_key_or_section() {
        let next = with_value(
            "[shell]\nthreshold_ms = 1\n",
            "general.autostart",
            Val::B(true),
        )
        .unwrap();
        assert!(parse(&next).unwrap().general.autostart);
        let next = with_value(
            "[general]\nadaptive_glow = false\n",
            "general.autostart",
            Val::B(true),
        )
        .unwrap();
        assert!(parse(&next).unwrap().general.autostart);
        let next = with_value("[general]\n", "general.theme", Val::S("light")).unwrap();
        assert_eq!(parse(&next).unwrap().general.theme, "light");
        let next = with_value("", "general.finish_peek_secs", Val::I(5)).unwrap();
        assert_eq!(parse(&next).unwrap().general.finish_peek_secs, 5);
        let next = with_value("", "general.glow_strength", Val::F(0.5)).unwrap();
        assert_eq!(parse(&next).unwrap().general.glow_strength, 0.5);
    }

    #[test]
    fn invalid_toml_is_rejected_not_defaulted() {
        assert!(parse("[general\nautostart = ").is_none());
        assert!(with_value("[general\n", "general.autostart", Val::B(true)).is_none());
        assert!(parse("").is_some(), "an empty file is all defaults");
    }

    #[test]
    fn hotkey_parsing() {
        assert_eq!(
            parse_hotkey("Win+Alt+N"),
            Some((MOD_WIN | MOD_ALT, b'N' as u32))
        );
        assert_eq!(
            parse_hotkey("ctrl + shift + f9"),
            Some((MOD_CONTROL | MOD_SHIFT, 0x78))
        );
        assert_eq!(parse_hotkey("Alt+1"), Some((MOD_ALT, b'1' as u32)));
        // Needs a modifier, one key, and a key we know.
        assert_eq!(parse_hotkey("N"), None);
        assert_eq!(parse_hotkey("Alt"), None);
        assert_eq!(parse_hotkey("Alt+N+M"), None);
        assert_eq!(parse_hotkey("Alt+F25"), None);
        assert_eq!(parse_hotkey("Alt+Enter"), None);
        assert_eq!(parse_hotkey(""), None);
    }

    #[test]
    fn old_fullscreen_keys_carry_over() {
        let f = |t: &str| parse(t).unwrap().general.fullscreen;
        assert_eq!(f(""), "line");
        assert_eq!(f("[general]\nsuppress_fullscreen = false\n"), "off");
        assert_eq!(f("[general]\nfullscreen_mode = \"hide\"\n"), "hide");
        assert_eq!(f("[general]\nfullscreen_mode = \"sentinel\"\n"), "line");
        // The new key wins over leftovers.
        let both = "[general]\nfullscreen = \"line\"\nfullscreen_mode = \"hide\"\n";
        assert_eq!(f(both), "line");
    }

    #[test]
    fn missing_keys_take_their_defaults() {
        let cfg = parse("[general]\ntheme = \"light\"\n[timer]\nrounds = 6\n").unwrap();
        assert_eq!(cfg.general.theme, "light");
        assert_eq!(cfg.timer.rounds, 6);
        let d = Config::default();
        assert_eq!(cfg.general.glow_strength, d.general.glow_strength);
        assert_eq!(cfg.timer.break_min, d.timer.break_min);
        assert_eq!(cfg.shell, d.shell);
        // A key that no longer exists is ignored, not an error.
        let gone = "[general]\nrenderer = \"classic\"\nsolid_pill = true\nacrylic = true\n";
        assert!(parse(gone).is_some());
    }
}
