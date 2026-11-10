//! Pane packs: `%APPDATA%\Hytte\panes\<name>\pane.toml`, a script pane and the actions it may
//! run, shared as a folder instead of pasted into `config.toml`.
//!
//! ```toml
//! name = "GitHub PRs"            # the tab (default: the folder's name)
//! command = "powershell -File prs.ps1"   # runs in this folder; prints text or {"rows": [...]}
//! every = 300
//!
//! [[action]]                      # rows may name it ("action": "open"); Hytte calls it github-prs-open
//! name = "open"
//! command = "gh pr view --web"
//! ```
//!
//! A pack runs nothing until you say so: the first time (and whenever its `pane.toml` changes) a
//! card shows its command with **Run** and **Ignore**. The answer is written beside it
//! (`.approved` / `.ignored`, holding a hash of the file), so an edited file asks again. A pack
//! shares the four-pane limit with `[[script_pane]]`s, which come first.

use crate::config::{
    NamedAction, ScriptPane, ACTION_MAX_TIMEOUT, ACTION_TIMEOUT, MAX_SCRIPT_PANES,
    SCRIPT_MIN_EVERY, SCRIPT_NAME_CHARS,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Actions one pack may bring.
pub const MAX_PACK_ACTIONS: usize = 8;
/// Longest pack folder name used as an action prefix.
const PREFIX_CHARS: usize = 20;

/// Where the packs live.
pub fn dir() -> PathBuf {
    crate::config::data_dir().join("panes")
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// You said Run for this exact `pane.toml`.
    Approved,
    /// New, or edited since: waiting for your answer.
    Pending,
    /// You said Ignore for this exact `pane.toml`.
    Ignored,
    /// `pane.toml` has a mistake; the text says what.
    Bad(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    /// The folder's name.
    pub folder: String,
    pub path: PathBuf,
    /// Hash of `pane.toml`, what an answer is tied to.
    pub hash: String,
    pub state: State,
    /// Tab label (empty when `Bad`).
    pub name: String,
    pub command: String,
    pub every: u64,
    /// Its actions, already named `<prefix>-<name>` and running in the pack's folder.
    pub actions: Vec<NamedAction>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    name: String,
    command: String,
    every: Option<u64>,
    action: Vec<NamedAction>,
}

/// FNV-1a over the file's text, as 16 hex digits: stable across Rust versions, unlike `Hasher`.
pub fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The folder name as the prefix of its actions: letters, digits, `-` and `_` (anything else
/// becomes `-`), lower case, at most [`PREFIX_CHARS`] characters.
pub fn prefix(folder: &str) -> String {
    folder
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(PREFIX_CHARS)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// A pack from the text of its `pane.toml`. `state` is for a good file: whether you have
/// answered. A mistake is `State::Bad` with the reason.
pub fn parse(folder: &str, path: &Path, text: &str, answer: Option<(bool, String)>) -> Pack {
    let h = hash(text);
    let bad = |why: String| Pack {
        folder: folder.into(),
        path: path.to_path_buf(),
        hash: h.clone(),
        state: State::Bad(why),
        name: String::new(),
        command: String::new(),
        every: 0,
        actions: vec![],
    };
    let f: File = match toml::from_str(text) {
        Ok(f) => f,
        Err(e) => return bad(e.message().to_string()),
    };
    let command = f.command.trim().to_string();
    if command.is_empty() {
        return bad("no `command`: a pack has to print something".into());
    }
    let pre = prefix(folder);
    if pre.is_empty() {
        return bad("the folder's name has no letters or digits".into());
    }
    let mut actions: Vec<NamedAction> = vec![];
    for a in f.action.into_iter().take(MAX_PACK_ACTIONS) {
        let name = a.name.trim().to_string();
        if !crate::scriptpane::valid_action_name(&name) || name.chars().count() > 19 {
            return bad(format!(
                "action \"{name}\": a name is letters, digits, - and _, up to 19 characters"
            ));
        }
        if a.command.trim().is_empty() {
            return bad(format!("action \"{name}\" has no command"));
        }
        let full = format!("{pre}-{name}");
        if actions.iter().any(|b| b.name.eq_ignore_ascii_case(&full)) {
            return bad(format!("two actions are called \"{name}\""));
        }
        actions.push(NamedAction {
            name: full,
            command: a.command.trim().to_string(),
            confirm: a.confirm,
            timeout: if a.timeout == 0 {
                ACTION_TIMEOUT
            } else {
                a.timeout.clamp(1, ACTION_MAX_TIMEOUT)
            },
            dir: Some(path.to_path_buf()),
        });
    }
    let name: String = if f.name.trim().is_empty() {
        folder.trim().to_string()
    } else {
        f.name.trim().to_string()
    }
    .chars()
    .take(SCRIPT_NAME_CHARS)
    .collect();
    let state = match answer {
        Some((true, a)) if a == h => State::Approved,
        Some((false, a)) if a == h => State::Ignored,
        _ => State::Pending,
    };
    Pack {
        folder: folder.into(),
        path: path.to_path_buf(),
        hash: h,
        state,
        name,
        command,
        every: f.every.unwrap_or(300).max(SCRIPT_MIN_EVERY),
        actions,
    }
}

/// Every pack in the folder, by folder name. Unreadable folders and folders without a
/// `pane.toml` are skipped.
pub fn scan() -> Vec<Pack> {
    let Ok(rd) = std::fs::read_dir(dir()) else {
        return vec![];
    };
    let mut out: Vec<Pack> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let path = e.path();
            let text = std::fs::read_to_string(path.join("pane.toml")).ok()?;
            let folder = e.file_name().to_string_lossy().into_owned();
            let read = |f: &str| std::fs::read_to_string(path.join(f)).ok();
            let answer = read(".approved")
                .map(|a| (true, a.trim().to_string()))
                .or_else(|| read(".ignored").map(|a| (false, a.trim().to_string())));
            Some(parse(&folder, &path, &text, answer))
        })
        .collect();
    out.sort_by_key(|p| p.folder.to_lowercase());
    out
}

/// Remember your answer for this exact file (its hash).
pub fn answer(pack: &Path, hash: &str, run: bool) {
    let (keep, drop) = if run {
        (".approved", ".ignored")
    } else {
        (".ignored", ".approved")
    };
    let _ = std::fs::remove_file(pack.join(drop));
    let _ = std::fs::write(pack.join(keep), hash);
}

/// What Hytte uses: `[[script_pane]]`s first, then the approved packs while there is room (a
/// pack whose tab name is already taken is skipped), and every action: those of
/// `[[action]]` and of approved packs, whose names carry the pack's prefix.
pub fn effective(
    config_panes: Vec<ScriptPane>,
    config_actions: Vec<NamedAction>,
    packs: &[Pack],
) -> (Vec<ScriptPane>, Vec<NamedAction>) {
    let mut panes = config_panes;
    let mut actions = config_actions;
    for p in packs.iter().filter(|p| p.state == State::Approved) {
        if panes.len() >= MAX_SCRIPT_PANES
            || panes.iter().any(|q| q.name.eq_ignore_ascii_case(&p.name))
        {
            continue;
        }
        panes.push(ScriptPane {
            name: p.name.clone(),
            command: p.command.clone(),
            every: p.every,
            pack: Some(prefix(&p.folder)),
            dir: Some(p.path.clone()),
        });
        for a in &p.actions {
            if !actions.iter().any(|b| b.name.eq_ignore_ascii_case(&a.name)) {
                actions.push(a.clone());
            }
        }
    }
    (panes, actions)
}

/// Re-scan when anything under the panes folder changes, and send the packs to the UI when they
/// differ from last time. The thread sleeps on a directory notification: no change, no wake-up.
pub fn spawn_watcher(ui_tx: crossbeam_channel::Sender<crate::ui_state::UiEvent>) {
    let _ = std::fs::create_dir_all(dir());
    let first = scan();
    let _ = ui_tx.send(crate::ui_state::UiEvent::Packs(first.clone()));
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
            &HSTRING::from(dir().as_os_str()),
            true,
            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
        ) else {
            return;
        };
        let mut last = first;
        while WaitForSingleObject(h, INFINITE) == WAIT_OBJECT_0 {
            // Editors and copies arrive in bursts: let them finish.
            std::thread::sleep(std::time::Duration::from_millis(300));
            if FindNextChangeNotification(h).is_err() {
                return;
            }
            let now = scan();
            if now != last {
                if ui_tx
                    .send(crate::ui_state::UiEvent::Packs(now.clone()))
                    .is_err()
                {
                    return;
                }
                last = now;
            }
        }
    });
    #[cfg(not(windows))]
    let _ = ui_tx;
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
        name = "GitHub PRs"
        command = "powershell -File prs.ps1"
        every = 120

        [[action]]
        name = "open"
        command = "gh pr view --web"
        confirm = true
    "#;

    fn pack(folder: &str, text: &str, answer: Option<(bool, String)>) -> Pack {
        parse(
            folder,
            &PathBuf::from(format!(r"C:\p\{folder}")),
            text,
            answer,
        )
    }

    #[test]
    fn a_good_pack_loads_with_prefixed_actions_and_waits_for_you() {
        let p = pack("github-prs", GOOD, None);
        assert_eq!(p.state, State::Pending);
        assert_eq!((p.name.as_str(), p.every), ("GitHub PRs", 120));
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].name, "github-prs-open");
        assert!(p.actions[0].confirm);
        assert_eq!(
            p.actions[0].dir.as_deref(),
            Some(Path::new(r"C:\p\github-prs"))
        );
        // Name and interval default sensibly.
        let q = pack("Docker Boxes", "command = \"docker ps\"", None);
        assert_eq!((q.name.as_str(), q.every), ("Docker Boxes", 300));
        assert_eq!(prefix("Docker Boxes"), "docker-boxes");
    }

    #[test]
    fn your_answer_holds_only_for_the_file_you_saw() {
        let h = hash(GOOD);
        assert_eq!(
            pack("p", GOOD, Some((true, h.clone()))).state,
            State::Approved
        );
        assert_eq!(
            pack("p", GOOD, Some((false, h.clone()))).state,
            State::Ignored
        );
        // The file was edited since: asked again.
        let edited = GOOD.replace("120", "60");
        assert_eq!(pack("p", &edited, Some((true, h))).state, State::Pending);
        assert_ne!(hash(GOOD), hash(&edited));
        assert_eq!(hash("a"), hash("a"));
    }

    #[test]
    fn a_bad_pack_says_what_is_wrong() {
        for (text, needle) in [
            ("command = [", ""),
            ("name = \"x\"", "no `command`"),
            ("command = \"x\"\n[[action]]\nname = \"has space\"\ncommand = \"y\"", "has space"),
            ("command = \"x\"\n[[action]]\nname = \"a\"", "no command"),
            ("command = \"x\"\n[[action]]\nname = \"a\"\ncommand = \"y\"\n[[action]]\nname = \"A\"\ncommand = \"z\"", "two actions"),
        ] {
            match pack("p", text, None).state {
                State::Bad(why) => assert!(why.contains(needle), "{text}: {why}"),
                s => panic!("{text}: {s:?}"),
            }
        }
        assert!(matches!(
            pack("!!!", "command = \"x\"", None).state,
            State::Bad(_)
        ));
    }

    #[test]
    fn packs_follow_the_config_panes_and_share_the_four_pane_limit() {
        let cfg = |n: &str| ScriptPane {
            name: n.into(),
            command: "x".into(),
            ..Default::default()
        };
        let approved = |folder: &str| {
            let t = "command = \"x\"\n[[action]]\nname = \"go\"\ncommand = \"y\"".to_string();
            let h = hash(&t);
            pack(folder, &t, Some((true, h)))
        };
        let packs = vec![
            approved("a"),
            pack("pending", GOOD, None),
            approved("b"),
            approved("c"),
        ];
        // One config pane leaves room for three: a, b and c. The pending one takes no slot.
        let (panes, actions) = effective(vec![cfg("Mine")], vec![], &packs);
        let names: Vec<&str> = panes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mine", "a", "b", "c"]);
        assert_eq!(panes[1].pack.as_deref(), Some("a"));
        let an: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(an, ["a-go", "b-go", "c-go"]);
        // Config panes come first and use up the room.
        let four = vec![cfg("1"), cfg("2"), cfg("3"), cfg("4")];
        let (panes, _) = effective(four, vec![], &packs);
        assert_eq!(panes.len(), 4);
        assert!(panes.iter().all(|p| p.pack.is_none()));
        // A tab name that is taken is skipped.
        let (panes, _) = effective(vec![cfg("A")], vec![], &packs[..1]);
        assert_eq!(panes.len(), 1);
    }

    #[test]
    fn packs_come_from_folders_and_answers_are_saved_beside_them() {
        let root = std::env::temp_dir().join(format!("hytte-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let one = root.join("one");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::write(one.join("pane.toml"), GOOD).unwrap();
        std::fs::create_dir_all(root.join("no-toml")).unwrap();
        // `scan` reads the real folder; test the pieces it is made of.
        let text = std::fs::read_to_string(one.join("pane.toml")).unwrap();
        let p = parse("one", &one, &text, None);
        assert_eq!(p.state, State::Pending);
        answer(&one, &p.hash, true);
        assert_eq!(
            std::fs::read_to_string(one.join(".approved")).unwrap(),
            p.hash
        );
        answer(&one, &p.hash, false);
        assert!(!one.join(".approved").exists() && one.join(".ignored").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
