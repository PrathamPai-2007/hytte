//! `hytte mcp grant <action>`, `revoke <action>` and `grants`: let an agent run one named action
//! without the Allow / Deny card each time. It is set here, in a terminal you typed into, and
//! never by a tool call. A grant is tied to the action's command: change the command and the
//! pill asks again. See `agent_guard.rs`.

use crate::agent_guard as g;

const HELP: &str = "\
usage:
  hytte mcp grant <action>    # an agent may run this action without asking you each time
  hytte mcp revoke <action>   # ask every time again
  hytte mcp grants            # what is allowed in advance";

/// The action with this name, as Hytte uses it (`config.toml`'s and approved packs').
fn find(name: &str) -> Option<crate::config::NamedAction> {
    let text = std::fs::read_to_string(crate::config::config_path()).unwrap_or_default();
    let cfg = crate::config::parse(&text).unwrap_or_default();
    crate::packs::effective(
        cfg.script_panes(),
        cfg.named_actions(),
        &crate::packs::scan(),
    )
    .1
    .into_iter()
    .find(|a| a.name.eq_ignore_ascii_case(name.trim()))
}

fn go(args: &[String]) -> Result<String, String> {
    let dir = g::grants_dir();
    let name = args.get(1).map(String::as_str).unwrap_or("");
    match args.first().map(String::as_str) {
        Some("grants") => {
            let v = g::granted_in(&dir);
            Ok(if v.is_empty() {
                "nothing is allowed in advance: every action_run asks you".into()
            } else {
                format!(
                    "allowed in advance (while the command is unchanged):\n{}",
                    v.join("\n")
                )
            })
        }
        Some("grant") => {
            let a = find(name).ok_or_else(|| {
                format!("no action called \"{name}\" in config.toml or an approved pack")
            })?;
            g::grant_in(
                &dir,
                &a.name,
                &g::grant_hash(&a.name, &a.command, a.dir.as_deref()),
            )?;
            Ok(format!(
                "An agent may now run \"{}\" without asking you:\n  {}\nIf the command changes, you are asked again. Undo with `hytte mcp revoke {}`.",
                a.name, a.command, a.name
            ))
        }
        Some("revoke") => Ok(if g::revoke_in(&dir, name) {
            format!("\"{name}\" asks you again.")
        } else {
            format!("\"{name}\" was not allowed in advance.")
        }),
        _ => Ok(HELP.into()),
    }
}

pub fn run(args: &[String]) {
    match go(args) {
        Ok(t) => println!("{t}"),
        Err(e) => {
            eprintln!("hytte mcp: {e}");
            std::process::exit(1);
        }
    }
}
