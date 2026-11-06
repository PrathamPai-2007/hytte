//! Hides likely secrets in text the pill draws (a command line, an output line, an agent's
//! question), so a token typed into a command doesn't land in a screen share. Display only:
//! Rerun and Copy keep the real text. A fixed list of common shapes, not a guarantee.

use std::borrow::Cow;

const MASK: &str = "•••";

/// Words that make the value next to them secret (`password=...`, `--api-key ...`).
const KEYS: [&str; 9] = [
    "password",
    "passwd",
    "pwd",
    "token",
    "secret",
    "apikey",
    "api_key",
    "api-key",
    "credential",
];

/// Prefixes of well-known token formats (GitHub, GitLab, Slack, Anthropic / OpenAI, AWS).
const PREFIXES: [&str; 9] = [
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "glpat-",
    "xox",
    "sk-",
    "AKIA",
];

fn sensitive(key: &str) -> bool {
    let k = key
        .trim_matches(['"', '\'', '-', ' ', ':', '='])
        .to_ascii_lowercase();
    !k.is_empty() && (KEYS.iter().any(|s| k.contains(s)) || k == "authorization")
}

/// `text` with secrets replaced by `•••`; borrowed when there was nothing to hide.
pub fn mask(text: &str) -> Cow<'_, str> {
    let first = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(['"', '\''])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let mysql = matches!(
        first.trim_end_matches(".exe"),
        "mysql" | "mysqldump" | "mysqladmin" | "mariadb"
    );
    let auth_header = text.to_ascii_lowercase().contains("authorization");
    let (mut out, mut changed, mut hide) = (String::with_capacity(text.len()), false, false);
    let mut rest = text;
    while !rest.is_empty() {
        let ws = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        out.push_str(&rest[..ws]);
        rest = &rest[ws..];
        if rest.is_empty() {
            break;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let tok = &rest[..end];
        rest = &rest[end..];
        let scheme = matches!(tok.to_ascii_lowercase().as_str(), "bearer" | "basic");
        if hide {
            hide = false;
            // `Authorization: Bearer xyz`: the scheme word stays, the token after it goes.
            if scheme {
                out.push_str(tok);
                hide = true;
            } else {
                out.push_str(&hidden(tok));
                changed = true;
            }
            continue;
        }
        let (shown, next) = token(tok, mysql, scheme, auth_header);
        changed |= shown != tok;
        hide = next;
        out.push_str(&shown);
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

/// A secret word replaced by `•••`, keeping a quote or comma that closed it.
fn hidden(tok: &str) -> String {
    let body = tok.trim_end_matches(['"', '\'', ',', '}', ';']);
    let lead = if body.starts_with(['"', '\'']) {
        &body[..1]
    } else {
        ""
    };
    format!("{lead}{MASK}{}", &tok[body.len()..])
}

/// One word: what to show, and whether the word after it is the secret.
fn token(tok: &str, mysql: bool, scheme: bool, auth_header: bool) -> (Cow<'_, str>, bool) {
    let same = |next| (Cow::Borrowed(tok), next);
    if scheme {
        // "Bearer" always; "Basic" only in an Authorization header (it is an ordinary word).
        return same(tok.eq_ignore_ascii_case("bearer") || auth_header);
    }
    // user:password@host in a URL.
    if let Some(i) = tok.find("://") {
        let after = &tok[i + 3..];
        let auth_end = after.find(['/', '?', '#']).unwrap_or(after.len());
        if let Some(at) = after[..auth_end].rfind('@') {
            if let Some(colon) = after[..at].find(':') {
                let keep = i + 3 + colon + 1;
                let pw_end = i + 3 + at;
                return (
                    Cow::Owned(format!("{}{MASK}{}", &tok[..keep], &tok[pw_end..])),
                    false,
                );
            }
        }
    }
    // key=value or "key":"value" in one word.
    if let Some(i) = tok.find(['=', ':']) {
        let (key, sep, value) = (&tok[..i], &tok[i..=i], &tok[i + 1..]);
        if !key.contains(['/', '\\']) && sensitive(key) {
            let value = value.trim_start_matches(['=', ':']);
            if value.is_empty() {
                return same(true); // `password:` / `--token=` then the value in the next word
            }
            let quote = if value.starts_with(['"', '\'']) {
                &value[..1]
            } else {
                ""
            };
            let tail_at = value
                .trim_end_matches(['"', '\'', ',', '}', ';'])
                .len()
                .max(quote.len());
            return (
                Cow::Owned(format!("{key}{sep}{quote}{MASK}{}", &value[tail_at..])),
                false,
            );
        }
    }
    // mysql -pSECRET
    if mysql && tok.len() > 2 && tok.starts_with("-p") {
        return (Cow::Owned(format!("-p{MASK}")), false);
    }
    // `--password hunter2`
    if tok.starts_with('-') && sensitive(tok) {
        return same(true);
    }
    // A well-known token format, bare.
    let value_at = tok.rfind('=').map_or(0, |i| i + 1);
    let bare = tok[value_at..].trim_matches(['"', '\'']);
    if bare.len() >= 20 && PREFIXES.iter().any(|p| bare.starts_with(p)) {
        return (Cow::Owned(format!("{}{MASK}", &tok[..value_at])), false);
    }
    same(false)
}

#[cfg(test)]
mod tests {
    use super::mask;

    fn m(s: &str) -> String {
        mask(s).into_owned()
    }

    #[test]
    fn hides_the_usual_shapes() {
        assert_eq!(
            m("curl -H 'Authorization: Bearer abc.def.ghi' https://x"),
            "curl -H 'Authorization: Bearer •••' https://x"
        );
        assert_eq!(
            m("curl -H \"Authorization: Basic dXNlcjpwdw==\" x"),
            "curl -H \"Authorization: Basic •••\" x"
        );
        assert_eq!(
            m("deploy --password hunter2 --force"),
            "deploy --password ••• --force"
        );
        assert_eq!(
            m("login password=hunter2 user=me"),
            "login password=••• user=me"
        );
        assert_eq!(m("API_TOKEN=abc123 npm run x"), "API_TOKEN=••• npm run x");
        assert_eq!(m("--token=abc"), "--token=•••");
        assert_eq!(m(r#"{"password":"hunter2"}"#), r#"{"password":"•••"}"#);
        assert_eq!(m("password: hunter2"), "password: •••");
        assert_eq!(
            m("git clone https://me:s3cret@github.com/a/b"),
            "git clone https://me:•••@github.com/a/b"
        );
        assert_eq!(m("mysql -u root -pSECRET db"), "mysql -u root -p••• db");
        assert_eq!(
            m("export X=ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
            "export X=•••"
        );
        assert_eq!(m("echo ghp_abcdefghijklmnopqrstuvwxyz0123"), "echo •••");
    }

    #[test]
    fn leaves_everything_else_alone() {
        for s in [
            "cargo build --release",
            "gcc -pedantic main.c",
            "git clone https://github.com/a/b",
            "Basic usage of the tool",
            "ssh user@host",
            "echo hello: world",
            "see C:\\temp\\token-list.txt",
            "",
        ] {
            assert!(matches!(mask(s), std::borrow::Cow::Borrowed(_)), "{s}");
        }
        // Whitespace is kept as typed.
        assert_eq!(m("a  --token   x  b"), "a  --token   •••  b");
    }

    #[test]
    fn never_panics_on_odd_text() {
        // Multi-byte characters next to every marker the masker slices on.
        let bits = [
            "é", "日本", "😀", "=", ":", "://", "@", "\"", "'", "-", "--", "-p", "password",
            "token=", "Bearer", " ", "	", "x", "ghp_", "xox", "sk-", "/", "\\",
        ];
        let mut state = 12345u64;
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..12 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push_str(bits[(state >> 33) as usize % bits.len()]);
            }
            let _ = mask(&text);
            let _ = mask(&format!("mysql {text}"));
        }
    }
}
