//! Rerun a failed command: a new terminal window, in the folder it ran in, through the same
//! shell, running the same command line again. The new console opens in Windows Terminal
//! when that is the default terminal app.

/// What to start: the program, its arguments, and (for cmd, which parses its own command
/// line) a raw tail passed through untouched.
#[derive(Debug, PartialEq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub raw: Option<String>,
}

/// How to run `label` again. `source` is the task's ("shell" or "run"), `shell` the full
/// path of the shell that ran it (shell tasks only), `hytte` this executable's path.
pub fn plan(source: &str, label: &str, shell: Option<&str>, hytte: &str) -> Launch {
    let powershell = |line: &str| Launch {
        program: "powershell.exe".into(),
        // Encoded, so no quote or semicolon in the command is ever re-parsed on the way in.
        args: vec!["-NoExit".into(), "-EncodedCommand".into(), encode_ps(line)],
        raw: None,
    };
    if source == "run" {
        // `hytte run -- <label>`: tracked on the pill again, like the first time.
        return powershell(&format!("& '{}' run -- {label}", hytte.replace('\'', "''")));
    }
    let Some(path) = shell else {
        return powershell(label);
    };
    let name = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    let stem = name.trim_end_matches(".exe");
    match stem {
        "pwsh" | "powershell" => Launch {
            program: path.into(),
            ..powershell(label)
        },
        "cmd" => Launch {
            program: path.into(),
            args: vec![],
            raw: Some(format!("/k {label}")),
        },
        // Run it, then stay in an interactive shell of the same kind.
        "bash" | "zsh" | "sh" | "fish" => Launch {
            program: path.into(),
            args: vec!["-i".into(), "-c".into(), format!("{label}; exec {stem} -i")],
            raw: None,
        },
        "nu" => Launch {
            program: path.into(),
            args: vec!["-e".into(), label.into()],
            raw: None,
        },
        // A shell we don't know how to drive: PowerShell runs most one-liners the same.
        _ => powershell(label),
    }
}

/// PowerShell's `-EncodedCommand`: base64 of the UTF-16LE text.
fn encode_ps(s: &str) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() {
                B64[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Start it in a new console window in `cwd`.
#[cfg(windows)]
pub fn start(l: &Launch, cwd: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x10;
    let mut c = std::process::Command::new(&l.program);
    c.args(&l.args)
        .current_dir(cwd)
        .creation_flags(CREATE_NEW_CONSOLE);
    if let Some(r) = &l.raw {
        c.raw_arg(r);
    }
    c.spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_powershell() {
        // [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes("dir"))
        assert_eq!(encode_ps("dir"), "ZABpAHIA");
        assert_eq!(encode_ps("a"), "YQA=");
        assert_eq!(encode_ps(""), "");
    }

    #[test]
    fn each_shell_gets_its_own_way_in() {
        let h = r"C:\Tools\hytte.exe";
        let pw = plan(
            "shell",
            "npm test",
            Some(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            h,
        );
        assert_eq!(pw.program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(pw.args[..2], ["-NoExit", "-EncodedCommand"]);
        let cmd = plan(
            "shell",
            "make && echo ok",
            Some(r"C:\Windows\System32\cmd.exe"),
            h,
        );
        assert_eq!(cmd.raw.as_deref(), Some("/k make && echo ok"));
        // Git Bash, by its full path: never whatever `bash.exe` PATH finds first (WSL).
        let bash = plan(
            "shell",
            "cargo test",
            Some(r"C:\Program Files\Git\usr\bin\bash.exe"),
            h,
        );
        assert_eq!(bash.program, r"C:\Program Files\Git\usr\bin\bash.exe");
        assert_eq!(bash.args, ["-i", "-c", "cargo test; exec bash -i"]);
        let nu = plan("shell", "ls", Some(r"C:\nu\nu.exe"), h);
        assert_eq!(nu.args, ["-e", "ls"]);
        // `hytte run` tasks go back through `hytte run`, so the rerun is tracked too.
        let run = plan("run", "npm test", None, r"C:\it's\hytte.exe");
        assert_eq!(run.program, "powershell.exe");
        assert_eq!(
            run.args[2],
            encode_ps(r"& 'C:\it''s\hytte.exe' run -- npm test")
        );
        // An unknown shell, or no shell at all, falls back to PowerShell.
        assert_eq!(
            plan("shell", "x", Some(r"C:\weird.exe"), h).program,
            "powershell.exe"
        );
        assert_eq!(plan("shell", "x", None, h).program, "powershell.exe");
    }
}
