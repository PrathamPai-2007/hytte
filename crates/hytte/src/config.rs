//! Single TOML settings file in %APPDATA%\Hytte.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct General {
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "default_monitor")]
    pub monitor: String,
    #[serde(default = "default_true")]
    pub solid_pill: bool,
    #[serde(default)]
    pub acrylic: bool,
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
}

fn default_monitor() -> String {
    "primary".into()
}
fn default_true() -> bool {
    true
}
fn default_sentinel() -> String {
    "sentinel".into()
}

impl Default for General {
    fn default() -> Self {
        Self {
            autostart: false,
            monitor: "primary".into(),
            solid_pill: true,
            acrylic: false,
            suppress_fullscreen: true,
            fullscreen_mode: "sentinel".into(),
            output_folder: None,
            allow_list: vec![],
            deny_list: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    ["vim", "nvim", "vi", "nano", "less", "more", "man", "ssh", "top", "htop", "tmux", "fzf", "git-credential-manager"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}
impl Default for Shell {
    fn default() -> Self {
        Self { threshold_ms: d_threshold(), ignore: d_ignore() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        Self { mode: d_ref(), max_items: d_max(), persist: true, remove_after_drag: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        Self { watch: d_watch(), show_all: false, poll_secs: d_poll() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        Self { peek_secs: d_peek(), sound: false }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub shell: Shell,
    #[serde(default)]
    pub shelf: Shelf,
    #[serde(default)]
    pub ports: Ports,
    #[serde(default)]
    pub agent: Agent,
}

pub fn data_dir() -> PathBuf {
    config_path().parent().map(|p| p.to_path_buf()).unwrap_or_default()
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
    toml::from_str(&text).unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let text = toml::to_string_pretty(cfg).unwrap_or_default();
    std::fs::write(path, text)
}

/// Opt-in autostart via HKCU\...\Run. No-op off Windows.
pub fn ensure_autostart(enable: bool) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
            REG_SZ,
        };
        let sub: Vec<u16> = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run\0"
            .encode_utf16()
            .collect();
        let mut hkey = HKEY::default();
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, windows::core::PCWSTR(sub.as_ptr()), None, KEY_SET_VALUE, &mut hkey) == ERROR_SUCCESS {
                if enable {
                    if let Ok(exe) = std::env::current_exe() {
                        let v: Vec<u16> = format!("\"{}\"\0", exe.display()).encode_utf16().collect();
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
