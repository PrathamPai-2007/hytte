//! Single TOML settings file in %APPDATA%\Hytte.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct General {
    #[serde(default)]
    pub autostart: bool,
    /// Put the folder holding `notch.exe` on the user's PATH at startup.
    #[serde(default = "default_true")]
    pub add_to_path: bool,
    /// Which monitor the pill hangs from: "primary", "active" (the foreground window's) or
    /// "cursor" (under the pointer; re-checked when the foreground window changes).
    #[serde(default = "default_monitor")]
    pub monitor: String,
    #[serde(default = "default_true")]
    pub solid_pill: bool,
    #[serde(default)]
    pub acrylic: bool,
    /// Tint the media glow with the album art's dominant colour.
    #[serde(default = "default_true")]
    pub adaptive_glow: bool,
    /// How strong the border glow is: 1.0 is the original look, 0 turns it off.
    #[serde(default = "default_glow")]
    pub glow_strength: f32,
    /// Global hotkey that opens the pill for keyboard use, e.g. "Win+Alt+N". Empty = off.
    #[serde(default)]
    pub hotkey: String,
    /// "dark" (default), "light", or "auto" (follow the Windows app theme).
    #[serde(default = "default_theme")]
    pub theme: String,
    /// "gpu" (DirectComposition swap chain) or "classic" (layered window). Read at startup.
    #[serde(default = "default_renderer")]
    pub renderer: String,
    #[serde(default = "default_true")]
    pub suppress_fullscreen: bool,
    #[serde(default = "default_sentinel")]
    pub fullscreen_mode: String, // "hide" | "sentinel"
    #[serde(default)]
    pub output_folder: Option<String>,
    #[serde(default)]
    pub allow_list: Vec<String>,
    #[serde(default)]
    pub deny_list: Vec<String>,
    /// Seconds the pill opens to show a finished or failed task (0 = don't open).
    #[serde(default = "default_finish_peek")]
    pub finish_peek_secs: u64,
    /// Keep the pill out of screenshots, recordings and screen share.
    #[serde(default)]
    pub hide_from_capture: bool,
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

fn default_monitor() -> String {
    "primary".into()
}
fn default_true() -> bool {
    true
}
fn default_finish_peek() -> u64 {
    5
}
fn default_sentinel() -> String {
    "sentinel".into()
}
fn default_glow() -> f32 {
    1.2
}

fn default_theme() -> String {
    "dark".into()
}
fn default_renderer() -> String {
    "gpu".into()
}

impl Default for General {
    fn default() -> Self {
        Self {
            autostart: false,
            add_to_path: true,
            monitor: "primary".into(),
            solid_pill: true,
            acrylic: false,
            adaptive_glow: true,
            glow_strength: default_glow(),
            hotkey: String::new(),
            theme: default_theme(),
            renderer: default_renderer(),
            suppress_fullscreen: true,
            fullscreen_mode: "sentinel".into(),
            output_folder: None,
            allow_list: vec![],
            deny_list: vec![],
            finish_peek_secs: default_finish_peek(),
            hide_from_capture: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shell {
    /// Commands shorter than this never show (the daemon enforces it).
    #[serde(default = "d_threshold")]
    pub threshold_ms: u32,
    #[serde(default = "d_ignore")]
    pub ignore: Vec<String>,
}
fn d_threshold() -> u32 {
    3000
}
fn d_ignore() -> Vec<String> {
    [
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
    .collect()
}
impl Default for Shell {
    fn default() -> Self {
        Self {
            threshold_ms: d_threshold(),
            ignore: d_ignore(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    /// "reference" keeps the original path; "copy" stashes a copy in %APPDATA%\Hytte\shelf.
    #[serde(default = "d_ref")]
    pub mode: String,
    #[serde(default = "d_max")]
    pub max_items: usize,
    #[serde(default = "default_true")]
    pub persist: bool,
    #[serde(default = "default_true")]
    pub remove_after_drag: bool,
}
fn d_ref() -> String {
    "reference".into()
}
fn d_max() -> usize {
    12
}
impl Default for Shelf {
    fn default() -> Self {
        Self {
            mode: d_ref(),
            max_items: d_max(),
            persist: true,
            remove_after_drag: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ports {
    #[serde(default = "d_watch")]
    pub watch: Vec<u16>,
    #[serde(default)]
    pub show_all: bool,
    #[serde(default = "d_poll")]
    pub poll_secs: u64,
}
fn d_watch() -> Vec<u16> {
    vec![3000, 3001, 4200, 5000, 5173, 5432, 8000, 8080, 8888]
}
fn d_poll() -> u64 {
    5
}
impl Default for Ports {
    fn default() -> Self {
        Self {
            watch: d_watch(),
            show_all: false,
            poll_secs: d_poll(),
        }
    }
}

/// A heads-up before the next calendar event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Off by default: event titles are private and the pill is on screen. Read at startup.
    #[serde(default)]
    pub enabled: bool,
    /// Minutes before the start to show the heads-up.
    #[serde(default = "d_cal_lead")]
    pub lead_min: u32,
}
fn d_cal_lead() -> u32 {
    10
}
impl Default for Calendar {
    fn default() -> Self {
        Self {
            enabled: false,
            lead_min: d_cal_lead(),
        }
    }
}

/// Show browser downloads as tasks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Downloads {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Folder to watch; empty = the user's Downloads folder. Read at startup.
    #[serde(default)]
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
pub struct Hog {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Percent of the whole machine, all cores together.
    #[serde(default = "d_hog_cpu")]
    pub cpu_pct: u32,
    /// Working set in MB.
    #[serde(default = "d_hog_mem")]
    pub mem_mb: u64,
    /// How long the CPU has to stay over the limit.
    #[serde(default = "d_hog_secs")]
    pub secs: u64,
}
fn d_hog_cpu() -> u32 {
    80
}
fn d_hog_mem() -> u64 {
    4096
}
fn d_hog_secs() -> u64 {
    30
}
impl Default for Hog {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_pct: d_hog_cpu(),
            mem_mb: d_hog_mem(),
            secs: d_hog_secs(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    #[serde(default = "d_peek")]
    pub peek_secs: u64,
    #[serde(default)]
    pub sound: bool,
}
fn d_peek() -> u64 {
    6
}
impl Default for Agent {
    fn default() -> Self {
        Self {
            peek_secs: d_peek(),
            sound: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    #[serde(default = "default_true")]
    pub sound: bool,
    #[serde(default = "d_break")]
    pub break_min: u32,
    #[serde(default = "d_long")]
    pub long_break_min: u32,
    /// Focus sessions before a long break.
    #[serde(default = "d_rounds")]
    pub rounds: u32,
}
fn d_break() -> u32 {
    5
}
fn d_long() -> u32 {
    15
}
fn d_rounds() -> u32 {
    4
}
impl Default for Timer {
    fn default() -> Self {
        Self {
            sound: true,
            break_min: d_break(),
            long_break_min: d_long(),
            rounds: d_rounds(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub timer: Timer,
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub shell: Shell,
    #[serde(default)]
    pub shelf: Shelf,
    #[serde(default)]
    pub ports: Ports,
    #[serde(default)]
    pub hog: Hog,
    #[serde(default)]
    pub downloads: Downloads,
    #[serde(default)]
    pub calendar: Calendar,
    #[serde(default)]
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
    toml::from_str(text).ok()
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

/// Sets `[general] autostart` in place, keeping the user's comments and layout. An unparsable
/// file is left alone (the setting still applies for this session).
pub fn set_autostart(enable: bool) -> std::io::Result<()> {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        let mut cfg = Config::default();
        cfg.general.autostart = enable;
        return save(&cfg);
    };
    match with_autostart(&text, enable) {
        Some(next) if next != text => write_atomic(&path, &next),
        _ => Ok(()),
    }
}

fn with_autostart(text: &str, enable: bool) -> Option<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().ok()?;
    let general = doc
        .entry("general")
        .or_insert_with(toml_edit::table)
        .as_table_like_mut()?;
    match general.get_mut("autostart").and_then(|i| i.as_value_mut()) {
        // Keep the spacing and any trailing comment on the line.
        Some(v) => {
            let decor = v.decor().clone();
            *v = enable.into();
            *v.decor_mut() = decor;
        }
        None => {
            general.insert("autostart", toml_edit::value(enable));
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
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_SET_VALUE, REG_SZ,
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
                KEY_SET_VALUE,
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
                    let name: Vec<u16> = "Hytte\0".encode_utf16().collect();
                    let _ = RegDeleteValueW(hkey, windows::core::PCWSTR(name.as_ptr()));
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
            eprintln!("hytte: start menu shortcut failed: {e}");
        }
        CoUninitialize();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_edit_keeps_comments_and_other_keys() {
        let text = "# my settings\n[general]\nautostart = false # login\nacrylic = true\n\n[shell]\n# keep me\nthreshold_ms = 5000\n";
        let next = with_autostart(text, true).unwrap();
        assert_eq!(next, text.replace("autostart = false", "autostart = true"));
        let cfg = parse(&next).unwrap();
        assert!(cfg.general.autostart && cfg.general.acrylic);
        assert_eq!(cfg.shell.threshold_ms, 5000);
    }

    #[test]
    fn autostart_edit_adds_missing_key_or_section() {
        let next = with_autostart("[shell]\nthreshold_ms = 1\n", true).unwrap();
        assert!(parse(&next).unwrap().general.autostart);
        let next = with_autostart("[general]\nacrylic = true\n", true).unwrap();
        assert!(parse(&next).unwrap().general.autostart);
    }

    #[test]
    fn invalid_toml_is_rejected_not_defaulted() {
        assert!(parse("[general\nautostart = ").is_none());
        assert!(with_autostart("[general\n", true).is_none());
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
}
