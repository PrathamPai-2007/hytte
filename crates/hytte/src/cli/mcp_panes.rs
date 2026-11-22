//! The pane tools of `hytte mcp`: an agent can make, change and remove a pane, but only as a
//! pane pack, so it arrives through the same gate as a downloaded one: nothing runs until you
//! press **Run** on the card that shows the command, and any edit asks again (the answer is
//! tied to the file's hash). An agent can only touch the packs it made itself (a marker file
//! says so); your own panes and packs are never changed. Every function takes the packs folder.

use crate::config::{SCRIPT_MIN_EVERY, SCRIPT_NAME_CHARS};
use std::path::{Path, PathBuf};

/// Present in a pack folder an agent made.
pub const MARKER: &str = ".hytte-agent";
/// How many panes agents may have at once.
pub const MAX_AGENT_PANES: usize = 4;
/// What the Run card shows of a command (`Ui::ask_packs` cuts at 60 characters). An agent's
/// command must fit, so the person presses Run on all of it, not on a harmless-looking start.
pub const MAX_COMMAND: usize = 60;
const MAX_EVERY: u64 = 86_400;

fn bad_text(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// The folder a pane lives in, from its name; Err when the name is unusable.
fn folder_of(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > SCRIPT_NAME_CHARS || bad_text(name) {
        return Err(format!(
            "name must be 1 to {SCRIPT_NAME_CHARS} characters, one line"
        ));
    }
    let f = crate::packs::prefix(name);
    if f.is_empty() {
        return Err("name needs at least one letter or digit".into());
    }
    Ok(f)
}

fn check_command(command: &str) -> Result<String, String> {
    if bad_text(command) {
        return Err("command must be one line".into());
    }
    // Runs of spaces collapse, so padding can't push the real command off the card.
    let c = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if c.is_empty() || c.chars().count() > MAX_COMMAND {
        return Err(format!(
            "command must be 1 to {MAX_COMMAND} characters on one line, so the user can read all of it on the Run card (it is run with cmd /c)"
        ));
    }
    Ok(c)
}

fn check_every(every: Option<u64>) -> Result<u64, String> {
    let e = every.unwrap_or(60);
    if (SCRIPT_MIN_EVERY..=MAX_EVERY).contains(&e) {
        Ok(e)
    } else {
        Err(format!(
            "every must be {SCRIPT_MIN_EVERY} to {MAX_EVERY} seconds"
        ))
    }
}

fn toml_of(name: &str, command: &str, every: u64) -> String {
    let mut t = toml::Table::new();
    t.insert("name".into(), name.trim().to_string().into());
    t.insert("command".into(), command.to_string().into());
    t.insert("every".into(), (every as i64).into());
    toml::to_string(&t).unwrap_or_default()
}

fn made_by_agent(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir.join(MARKER)).is_ok_and(|m| m.is_file())
}

fn agent_panes(packs: &Path) -> usize {
    std::fs::read_dir(packs)
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().is_dir() && made_by_agent(&e.path()))
                .count()
        })
        .unwrap_or(0)
}

fn write_pack(dir: &Path, name: &str, command: &str, every: u64) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create the folder: {e}"))?;
    std::fs::write(dir.join(MARKER), "made by an agent through hytte mcp\n")
        .map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pane.toml"), toml_of(name, command, every))
        .map_err(|e| format!("couldn't write the pane: {e}"))
}

const ASK: &str = "It is waiting for you: a card on the pill shows the command with Run and Ignore, and nothing runs until you press Run.";

/// `taken`: names of the panes that already exist (yours and packs'), compared without case.
pub fn create(
    packs: &Path,
    taken: &[String],
    name: &str,
    command: &str,
    every: Option<u64>,
) -> Result<String, String> {
    let folder = folder_of(name)?;
    let command = check_command(command)?;
    let every = check_every(every)?;
    if taken.iter().any(|t| t.eq_ignore_ascii_case(name.trim())) {
        return Err(format!(
            "a pane called \"{}\" already exists; use pane_update for one you made, or pick another name",
            name.trim()
        ));
    }
    let dir: PathBuf = packs.join(&folder);
    if dir.exists() {
        return Err(format!(
            "the folder \"{folder}\" already exists; use pane_update if you made it"
        ));
    }
    if agent_panes(packs) >= MAX_AGENT_PANES {
        return Err(format!(
            "at most {MAX_AGENT_PANES} panes made by agents; remove one with pane_remove first"
        ));
    }
    write_pack(&dir, name, &command, every)?;
    Ok(format!("Created the pane \"{}\". {ASK}", name.trim()))
}

pub fn update(
    packs: &Path,
    name: &str,
    command: &str,
    every: Option<u64>,
) -> Result<String, String> {
    let folder = folder_of(name)?;
    let command = check_command(command)?;
    let every = check_every(every)?;
    let dir = packs.join(&folder);
    if !made_by_agent(&dir) {
        return Err(format!(
            "no pane \"{}\" made by an agent: yours are never changed",
            name.trim()
        ));
    }
    write_pack(&dir, name, &command, every)?;
    Ok(format!(
        "Updated the pane \"{}\". Because it changed, it asks again. {ASK}",
        name.trim()
    ))
}

pub fn remove(packs: &Path, name: &str) -> Result<String, String> {
    let folder = folder_of(name)?;
    let dir = packs.join(&folder);
    if !made_by_agent(&dir) {
        return Err(format!(
            "no pane \"{}\" made by an agent: yours are never removed",
            name.trim()
        ));
    }
    // Only what this module writes; anything else in the folder means a person added to it.
    let ours = |n: &std::ffi::OsStr| n == MARKER || n == "pane.toml";
    let extra = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .any(|e| !ours(&e.file_name()));
    if extra {
        return Err("that folder has files besides the pane, so it is left alone".into());
    }
    for f in [MARKER, "pane.toml"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    std::fs::remove_dir(&dir).map_err(|e| format!("couldn't remove it: {e}"))?;
    Ok(format!("Removed the pane \"{}\".", name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hytte-mcppanes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_created_pane_is_a_valid_unapproved_pack() {
        let d = tmp("ok");
        create(&d, &[], "Build", "echo hi", Some(45)).unwrap();
        let text = std::fs::read_to_string(d.join("build").join("pane.toml")).unwrap();
        let p = crate::packs::parse("build", &d.join("build"), &text, None);
        assert_eq!(p.state, crate::packs::State::Pending, "needs your Run");
        assert_eq!(
            (p.name.as_str(), p.command.as_str(), p.every),
            ("Build", "echo hi", 45)
        );
        assert!(made_by_agent(&d.join("build")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn padding_cannot_hide_the_end_of_a_command() {
        let d = tmp("pad");
        // Spaces collapse, so what the card shows is the whole command.
        create(&d, &[], "Pad", "echo   hi     there", None).unwrap();
        let text = std::fs::read_to_string(d.join("pad").join("pane.toml")).unwrap();
        assert!(text.contains("echo hi there"), "{text}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_update_changes_the_hash_so_it_asks_again() {
        let d = tmp("upd");
        create(&d, &[], "Build", "echo one", None).unwrap();
        let read = || std::fs::read_to_string(d.join("build").join("pane.toml")).unwrap();
        let before = crate::packs::hash(&read());
        update(&d, "Build", "echo two", None).unwrap();
        assert_ne!(before, crate::packs::hash(&read()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn it_refuses_bad_input_and_other_peoples_panes() {
        let d = tmp("bad");
        for (n, c, e) in [
            ("", "x", None),
            ("a\nb", "x", None),
            ("name-that-is-too-long", "x", None),
            ("---", "x", None),
            ("ok", "", None),
            ("ok", "a\nb", None),
            ("ok", &"x".repeat(61), None),
            (
                "ok",
                &format!("echo ok{}& {}", " ".repeat(70), "x".repeat(60)),
                None,
            ),
            ("ok", "x", Some(5)),
            ("ok", "x", Some(999_999)),
        ] {
            assert!(create(&d, &[], n, c, e).is_err(), "{n:?} {c:?} {e:?}");
        }
        // Your [[script_pane]] or pack with that name.
        assert!(create(&d, &["Build".into()], "build", "x", None).is_err());
        // A pack you made by hand, without the marker, is never touched.
        std::fs::create_dir_all(d.join("mine")).unwrap();
        std::fs::write(d.join("mine").join("pane.toml"), "name='Mine'\ncommand='x'").unwrap();
        assert!(update(&d, "mine", "evil", None).is_err());
        assert!(remove(&d, "mine").is_err());
        assert!(
            create(&d, &[], "mine", "x", None).is_err(),
            "the folder exists"
        );
        assert!(d.join("mine").join("pane.toml").exists());
        // A name can't climb out of the packs folder.
        assert!(remove(&d, "..").is_err());
        assert!(update(&d, r"..\x", "x", None).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn at_most_four_and_remove_frees_a_slot() {
        let d = tmp("cap");
        for i in 0..MAX_AGENT_PANES {
            create(&d, &[], &format!("p{i}"), "x", None).unwrap();
        }
        assert!(create(&d, &[], "p9", "x", None).is_err());
        remove(&d, "p0").unwrap();
        assert!(!d.join("p0").exists());
        create(&d, &[], "p9", "x", None).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn remove_leaves_a_folder_with_foreign_files() {
        let d = tmp("foreign");
        create(&d, &[], "Build", "x", None).unwrap();
        std::fs::write(d.join("build").join("notes.txt"), "mine").unwrap();
        assert!(remove(&d, "Build").is_err());
        assert!(d.join("build").join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
