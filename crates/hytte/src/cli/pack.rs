//! `hytte pack`: find, read about, install, update and remove shared pane packs.
//! The work is in `registry.rs`; this only reads the arguments and prints.

use crate::registry;

const HELP: &str = "\
usage:
  hytte pack search [word]        # packs in the registry
  hytte pack info <name>          # what a pack would run (read it before you install)
  hytte pack install <name>       # download, verify, put it in your panes folder
  hytte pack update [name]        # bring installed packs up to date (all when no name)
  hytte pack remove <name>        # remove a pack that `hytte pack install` made
  hytte pack list                 # what is installed
  --source <https-url|folder>     # use another registry (a folder with index.json) for this command
An installed pack still waits for your Run on the pill before anything runs.";

/// The packs and the other arguments of `hytte pack ...`, with `--source` taken out.
fn split(args: &[String]) -> Result<(Vec<String>, Option<String>), String> {
    let mut rest = vec![];
    let mut source = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--source" {
            source = Some(it.next().ok_or("--source needs a value")?.clone());
        } else {
            rest.push(a.clone());
        }
    }
    Ok((rest, source))
}

fn go(args: &[String]) -> Result<String, String> {
    let (rest, source) = split(args)?;
    let Some(verb) = rest.first().map(String::as_str) else {
        return Ok(HELP.into());
    };
    let name = rest.get(1).map(String::as_str);
    let packs = crate::packs::dir();
    let current = env!("CARGO_PKG_VERSION");
    let src = registry::source_from(source.as_deref());
    fn need<'a>(n: Option<&'a str>, verb: &str) -> Result<&'a str, String> {
        n.ok_or_else(|| format!("hytte pack {verb}: which pack?"))
    }
    match verb {
        "help" | "-h" | "--help" => Ok(HELP.into()),
        "list" => {
            let v = registry::installed(&packs);
            Ok(if v.is_empty() {
                "nothing installed with `hytte pack install`".into()
            } else {
                v.iter()
                    .map(|(n, v)| format!("{n} {v}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        "remove" => registry::remove(&packs, need(name, verb)?, &crate::packs::forget_answer),
        "search" => Ok(registry::search(
            &registry::load_index(&*src)?,
            name.unwrap_or(""),
        )),
        "info" => registry::info(&*src, &registry::load_index(&*src)?, need(name, verb)?),
        "install" => registry::install(
            &*src,
            &registry::load_index(&*src)?,
            &packs,
            need(name, verb)?,
            current,
            &crate::packs::forget_answer,
        ),
        "update" => registry::update(
            &*src,
            &registry::load_index(&*src)?,
            &packs,
            name,
            current,
            &crate::packs::forget_answer,
        ),
        _ => Err(format!("hytte pack: unknown command \"{verb}\"\n{HELP}")),
    }
}

pub fn run(args: &[String]) {
    match go(args) {
        Ok(text) => println!("{text}"),
        Err(e) => {
            eprintln!("hytte pack: {e}");
            std::process::exit(1);
        }
    }
}
