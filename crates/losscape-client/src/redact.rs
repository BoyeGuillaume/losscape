//! Hides values that look like secrets in what a run records, before it reaches the local log
//! or the server: string values under names like `api_key` or `HF_TOKEN`, and in any text,
//! well-known kinds of credentials, passwords in URLs and assignments like `password=...`.
//!
//! This catches what can be recognized, not everything: a password printed on its own looks
//! like any other word. It's meant to be linear in the length of the text (no regular
//! expressions), since all console output goes through it.

use std::borrow::Cow;

use serde_json::{Map, Value};

pub const HIDDEN: &str = "***";

/// Words that make a name secret. Whole words only: `max_tokens`, `tokenizer` and
/// `keyframe` are fine; `api_key`, `hfToken` and `AWS_SECRET_ACCESS_KEY` are not.
const SECRET_WORDS: &[&str] = &[
    "key",
    "apikey",
    "token",
    "secret",
    "secrets",
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "credential",
    "credentials",
    "pass",
    "auth",
    "authorization",
    "cookie",
    "cookies",
    "dsn",
    "jwt",
    "bearer",
    "secretkey",
    "accesskey",
    "privatekey",
    "authtoken",
];

/// Words that make `name=value` in running text an assignment of a secret. Fewer than above:
/// text says `key=loss` and `key: 3` all the time.
const ASSIGNED_SECRET_WORDS: &[&str] = &[
    "token",
    "secret",
    "secrets",
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "credential",
    "credentials",
    "apikey",
    "secretkey",
    "accesskey",
    "privatekey",
    "authtoken",
    "authorization",
    "dsn",
];
/// `key` counts after these: `api_key`, `AWS_SECRET_ACCESS_KEY`.
/// Too common a word to count without quotes: `first pass: done`, but not `"pass": "abc"`.
const QUOTED_SECRET_WORDS: &[&str] = &["pass"];
const KEY_QUALIFIERS: &[&str] = &[
    "api",
    "access",
    "private",
    "secret",
    "auth",
    "ssh",
    "signing",
    "encryption",
];

/// Prefixes of common credentials: GitHub, OpenAI and Stripe, Hugging Face, Slack,
/// GitLab, AWS access keys, losscape API keys.
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "sk-",
    "sk_live_",
    "rk_live_",
    "hf_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "glpat-",
    "AKIA",
    "ASIA",
    "df_",
    "pypi-",
    "npm_",
    "AIza",
];
/// Of those, the ones that also begin ordinary names, like `df_train_features_v2`. In running
/// text they have to look random as well.
const AMBIGUOUS_PREFIXES: &[&str] = &["sk-", "hf_", "df_"];
const AWS_PREFIXES: &[&str] = &["AKIA", "ASIA"];

/// Lowercase words of a name: split at non-alphanumerics and lower-to-upper case changes.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut word = String::new();
        let mut prev_lower = false;
        for c in part.chars() {
            if c.is_ascii_uppercase() && prev_lower && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            word.push(c.to_ascii_lowercase());
        }
        if !word.is_empty() {
            out.push(word);
        }
    }
    out
}

pub fn secret_name(name: &str) -> bool {
    words(name)
        .iter()
        .any(|w| SECRET_WORDS.contains(&w.as_str()))
}

pub fn looks_like_token(value: &str) -> bool {
    let token_chars = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    };
    if TOKEN_PREFIXES
        .iter()
        .any(|p| value.starts_with(p) && value.len() >= p.len() + 16 && token_chars(value))
    {
        return true;
    }
    // JSON web tokens: three base64url parts, the first one a JSON object.
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && value.starts_with("eyJ")
        && value.len() > 40
        && parts.iter().all(|p| !p.is_empty() && token_chars(p))
}

/// A credential within running text, given a word of it.
fn embedded_token(word: &str) -> bool {
    if AWS_PREFIXES.iter().any(|p| word.starts_with(p)) {
        return word.len() == 20
            && word
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    }
    if AMBIGUOUS_PREFIXES.iter().any(|p| word.starts_with(p)) {
        return word.len() >= 24
            && word.chars().any(|c| c.is_ascii_lowercase())
            && word.chars().any(|c| c.is_ascii_uppercase())
            && word.chars().any(|c| c.is_ascii_digit());
    }
    looks_like_token(word)
}

/// Where a credential starts in a word, up to its end: also after a `-` or `_`, as in
/// `prod-ghp_abc...`. Past the start of a word, only prefixes that can't be anything else.
fn token_within(word: &str) -> Option<usize> {
    if embedded_token(word) {
        return Some(0);
    }
    word.match_indices(['-', '_']).find_map(|(at, _)| {
        let rest = &word[at + 1..];
        let distinctive = TOKEN_PREFIXES
            .iter()
            .filter(|p| !AMBIGUOUS_PREFIXES.contains(p) && !AWS_PREFIXES.contains(p))
            .any(|p| rest.starts_with(p));
        (distinctive && looks_like_token(rest)).then_some(at + 1)
    })
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Replaces the credentials among the words of `text`.
fn scrub_words(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut copied = 0;
    let mut rest_start = 0;
    while rest_start < text.len() {
        let rest = &text[rest_start..];
        let Some(start) = rest.find(word_char) else {
            break;
        };
        let word = &rest[start..];
        let len = word.find(|c| !word_char(c)).unwrap_or(word.len());
        let word = &word[..len];
        let begin = rest_start + start;
        // A JSON web token has dots in it; otherwise a dot ends a word.
        let whole = word.trim_end_matches(['.', '-', '_']);
        let mut part = 0;
        if whole.starts_with("eyJ") && looks_like_token(whole) {
            out.push_str(&text[copied..begin]);
            out.push_str(HIDDEN);
            copied = begin + whole.len();
        } else {
            for piece in whole.split('.') {
                if let Some(at) = token_within(piece) {
                    out.push_str(&text[copied..begin + part + at]);
                    out.push_str(HIDDEN);
                    copied = begin + part + piece.len();
                }
                part += piece.len() + 1;
            }
        }
        rest_start = begin + len;
    }
    if copied == 0 && out.is_empty() {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// `scheme://user:password@host` -> `scheme://user:***@host`.
fn scrub_urls(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find("://") {
        let start = from + found + 3;
        let rest = &text[start..];
        // The password can have an `@` in it: the host is what follows the last one.
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '"' | '\'' | '?' | '#' | ','))
            .unwrap_or(rest.len());
        let authority = &rest[..end];
        if let Some((user_info, _)) = authority.rsplit_once('@')
            && let Some((user, password)) = user_info.split_once(':')
            && !password.is_empty()
            && password != HIDDEN
        {
            out.push_str(&text[copied..start + user.len() + 1]);
            out.push_str(HIDDEN);
            copied = start + user_info.len();
        }
        from = start + end;
    }
    if out.is_empty() {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// A name that says what's assigned to it is secret.
fn assigned_secret(name: &str) -> bool {
    let words = words(name);
    words.iter().enumerate().any(|(i, w)| {
        ASSIGNED_SECRET_WORDS.contains(&w.as_str())
            || (w == "key" && i > 0 && KEY_QUALIFIERS.contains(&words[i - 1].as_str()))
    })
}

/// What's on the right of `password: str`, `token=5` or `secret = None` says nothing.
fn harmless_value(value: &str) -> bool {
    value.is_empty()
        || value.ends_with(HIDDEN)
        || value.parse::<f64>().is_ok()
        || matches!(
            value,
            "str" | "string" | "bytes" | "None" | "null" | "true" | "false" | "True" | "False"
        )
}

/// `password=abc`, `"api_key": "abc"`, `token: abc`, `Authorization: Basic abc`. On the right
/// of ` = `, only what's in quotes: unquoted that's code, like `token = encode(text)`.
fn scrub_assignments(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        let op = bytes[i];
        if op != b'=' && op != b':' {
            i += 1;
            continue;
        }
        let next = bytes.get(i + 1).copied();
        let previous = i.checked_sub(1).map(|p| bytes[p]);
        // Not `==`, `!=`, `<=`, `>=`, `://`, `::`.
        if (op == b'='
            && (next == Some(b'=') || matches!(previous, Some(b'=' | b'!' | b'<' | b'>'))))
            || (op == b':' && matches!(next, Some(b'/' | b':')))
        {
            i += 1;
            continue;
        }
        // The name on the left, perhaps in quotes.
        let mut end = i;
        while end > 0 && bytes[end - 1] == b' ' {
            end -= 1;
        }
        let spaced = end != i;
        // `config["password"] = ...`
        if end > 0 && bytes[end - 1] == b']' {
            end -= 1;
        }
        if end > 0 && matches!(bytes[end - 1], b'"' | b'\'') {
            end -= 1;
        }
        let mut start = end;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || matches!(bytes[start - 1], b'_' | b'-' | b'.'))
        {
            start -= 1;
        }
        let name = &text[start..end];
        let only_quoted = words(name)
            .iter()
            .any(|w| QUOTED_SECRET_WORDS.contains(&w.as_str()));
        if name.is_empty() || !(assigned_secret(name) || only_quoted) {
            i += 1;
            continue;
        }
        // The value on the right.
        let mut value_start = i + 1;
        while bytes.get(value_start) == Some(&b' ') {
            value_start += 1;
        }
        let quote = bytes
            .get(value_start)
            .copied()
            .filter(|b| matches!(b, b'"' | b'\''));
        let (from, to) = if let Some(quote) = quote {
            let from = value_start + 1;
            let closing = bytes[from..]
                .iter()
                .position(|&b| b == quote || b == b'\n')
                .map_or(bytes.len(), |n| from + n);
            (from, closing)
        } else if only_quoted || (op == b'=' && (spaced || value_start != i + 1)) {
            i += 1;
            continue;
        } else if words(name).iter().any(|w| w == "authorization") {
            // A scheme and a credential: to the end of the line, or of the quoted header.
            let end = bytes[value_start..]
                .iter()
                .position(|&b| matches!(b, b'\n' | b'"' | b'\''))
                .map_or(bytes.len(), |n| value_start + n);
            (value_start, end)
        } else {
            let end = bytes[value_start..]
                .iter()
                .position(|&b| {
                    b.is_ascii_whitespace() || matches!(b, b'&' | b',' | b';' | b')' | b'"' | b'\'')
                })
                .map_or(bytes.len(), |n| value_start + n);
            (value_start, end)
        };
        if !harmless_value(&text[from..to]) {
            out.push_str(&text[copied..from]);
            out.push_str(HIDDEN);
            copied = to;
        }
        i = to.max(i + 1);
    }
    if out.is_empty() {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// `Bearer abc...`, in whatever case, wherever it is.
fn scrub_bearer(text: &str) -> Cow<'_, str> {
    const WORD: &str = "bearer ";
    let mut out = String::new();
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = text.get(from..).and_then(|rest| {
        rest.as_bytes()
            .windows(WORD.len())
            .position(|w| w.eq_ignore_ascii_case(WORD.as_bytes()))
    }) {
        let start = from + found + WORD.len();
        let rest = &text[start..];
        let len = rest
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '=' | '~'))
            })
            .unwrap_or(rest.len());
        if len >= 16 {
            out.push_str(&text[copied..start]);
            out.push_str(HIDDEN);
            copied = start + len;
        }
        from = start + len;
    }
    if out.is_empty() {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// Hides what can be recognized as a secret in any text.
pub fn scrub(text: &str) -> Cow<'_, str> {
    let mut clean = Cow::Borrowed(text);
    for pass in [scrub_urls, scrub_bearer, scrub_assignments, scrub_words] {
        if let Cow::Owned(changed) = pass(&clean) {
            clean = Cow::Owned(changed);
        }
    }
    clean
}

/// [`scrub`] in place.
pub fn scrub_in_place(text: &mut String) {
    if let Cow::Owned(clean) = scrub(text) {
        *text = clean;
    }
}

/// Hides the value of an environment variable or argument called `name` if needed.
pub fn redact_str(name: &str, value: &str) -> String {
    if (secret_name(name) && !value.is_empty()) || looks_like_token(value) {
        HIDDEN.to_owned()
    } else {
        scrub(value).into_owned()
    }
}

/// Hides secrets in a JSON object, recursively. Numbers and booleans under secret names
/// are kept (`key_dim: 64` is a hyperparameter, not a secret).
pub fn redact_json(map: &mut Map<String, Value>) {
    for (name, value) in map.iter_mut() {
        redact_value(secret_name(name), value);
    }
}

/// `secret`: under a secret name. Lists and objects inherit that: `api_keys: [...]`,
/// `credentials: {user: ..., pass: ...}`.
fn redact_value(secret: bool, value: &mut Value) {
    match value {
        Value::String(s) => {
            if (secret && !s.is_empty()) || looks_like_token(s) {
                *s = HIDDEN.to_owned();
            } else {
                scrub_in_place(s);
            }
        }
        Value::Object(map) => {
            for (name, item) in map.iter_mut() {
                redact_value(secret || secret_name(name), item);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| redact_value(secret, v)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn secret_names_are_whole_words() {
        for name in [
            "api_key",
            "HF_TOKEN",
            "hfToken",
            "AWS_SECRET_ACCESS_KEY",
            "db-password",
            "apiKey",
        ] {
            assert!(secret_name(name), "{name}");
        }
        for name in [
            "max_tokens",
            "tokenizer",
            "keyframe",
            "monkey",
            "num_keys",
            "lr",
        ] {
            assert!(!secret_name(name), "{name}");
        }
    }

    #[test]
    fn known_token_shapes() {
        assert!(looks_like_token("ghp_0123456789abcdefghijABCDEFGHIJ"));
        assert!(looks_like_token("hf_abcdefghijklmnopqrstuvwxyz"));
        assert!(looks_like_token(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"
        ));
        assert!(!looks_like_token("hf_transfer"));
        assert!(!looks_like_token("sk-learn is great"));
        assert!(!looks_like_token("resnet50"));
    }

    #[test]
    fn redacts_config_values() {
        let mut config = json!({
            "lr": 0.001,
            "key_dim": 64,
            "tokenizer": "gpt2",
            "openai": {"api_key": "abc", "model": "gpt"},
            "wandb_keys": ["a", "b"],
            "note": "ghp_0123456789abcdefghijABCDEFGHIJ",
            "password": ""
        });
        redact_json(config.as_object_mut().unwrap());
        assert_eq!(
            config,
            json!({
                "lr": 0.001,
                "key_dim": 64,
                "tokenizer": "gpt2",
                "openai": {"api_key": "***", "model": "gpt"},
                "wandb_keys": ["a", "b"],
                "note": "***",
                "password": ""
            })
        );
    }

    const TOKEN: &str = "ghp_0123456789abcdefghijABCDEFGHIJ";

    #[test]
    fn secrets_in_objects_under_secret_names_are_hidden() {
        let mut config = json!({
            "credentials": {"user": "ada", "pass": "hunter2", "nested": {"x": "y", "n": 5}},
            "auth": "basic",
            "dsn": "postgres://u:p@h/db",
            "ok": {"dim": "64"},
        });
        redact_json(config.as_object_mut().unwrap());
        assert_eq!(
            config,
            json!({
                "credentials": {"user": "***", "pass": "***", "nested": {"x": "***", "n": 5}},
                "auth": "***",
                "dsn": "***",
                "ok": {"dim": "64"},
            })
        );
    }

    #[test]
    fn secrets_within_values_are_hidden() {
        let mut config = json!({
            "note": format!("Bearer {TOKEN}"),
            "url": "postgres://u:hunter2@db.example.com:5432/x?sslmode=require",
            "list": [format!("use {TOKEN} here")],
        });
        redact_json(config.as_object_mut().unwrap());
        assert_eq!(
            config,
            json!({
                "note": "Bearer ***",
                "url": "postgres://u:***@db.example.com:5432/x?sslmode=require",
                "list": ["use *** here"],
            })
        );
    }

    #[test]
    fn scrubs_running_text() {
        for (text, clean) in [
            // Credentials among other words, whatever surrounds them.
            (format!("token is {TOKEN}."), "token is ***."),
            (
                format!("https://{TOKEN}@github.com/a/b"),
                "https://***@github.com/a/b",
            ),
            (format!("x={TOKEN}&y=1"), "x=***&y=1"),
            ("AKIAIOSFODNN7EXAMPLE leaked".into(), "*** leaked"),
            // A key of ours, and the ambiguous prefixes when they look random.
            ("df_8fK2mQ9xLpR3vT7yZ1aB4cD6eG0hJ5nW".into(), "***"),
            ("hf_AbCdEfGhIjKlMnOpQrStUvWx1234".into(), "***"),
            // URLs.
            (
                "postgres://u:p@ss@host/db".into(),
                "postgres://u:***@host/db",
            ),
            (
                "see https://ada:s3cret@example.com/x now".into(),
                "see https://ada:***@example.com/x now",
            ),
            // Assignments.
            ("password=hunter2 and more".into(), "password=*** and more"),
            ("api_key: abc123".into(), "api_key: ***"),
            (
                r#"{"secret_key": "abc", "x": 1}"#.into(),
                r#"{"secret_key": "***", "x": 1}"#,
            ),
            ("TOKEN = \"abc\"".into(), "TOKEN = \"***\""),
            (
                "https://x.io/?a=1&token=abc&b=2".into(),
                "https://x.io/?a=1&token=***&b=2",
            ),
            ("--password=hunter2".into(), "--password=***"),
            (
                "Authorization: Basic dXNlcjpwYXNz\nnext".into(),
                "Authorization: ***\nnext",
            ),
            // Bearer tokens of any shape.
            (
                "curl -H 'authorization: bearer abcdefghij0123456789+/='".into(),
                "curl -H 'authorization: bearer ***'",
            ),
            ("BEARER abcdefghij0123456789".into(), "BEARER ***"),
            // Within a longer word.
            (format!("prod-{TOKEN}"), "prod-***"),
            (format!("my_token_{TOKEN} rest"), "my_token_*** rest"),
            // As written in code.
            (
                "config['password'] = 'abc'".into(),
                "config['password'] = '***'",
            ),
            (
                "os.environ[\"API_KEY\"]=\"abc\"".into(),
                "os.environ[\"API_KEY\"]=\"***\"",
            ),
            (r#"{"pass": "abc"}"#.into(), r#"{"pass": "***"}"#),
        ] {
            assert_eq!(scrub(&text), clean, "{text}");
        }
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "epoch 3: loss=0.25 acc=0.9",
            "key=val/loss key: 3",
            "max_tokens=512 tokenizer=gpt2 num_keys: 4",
            "token=15",
            "df_train_features_normalized_v2 = load()",
            "hf_transformer_cache_directory",
            "sk-learn-and-more-words-here-for-length",
            "https://github.com/a/b http://localhost:8080/x ssh://git@github.com/a",
            "token = tokenizer.encode(text)",
            "password = getpass()",
            "def login(password: str, token: None):",
            "if password == other:",
            "a::b c://d",
            "Bearer short",
            "the bearer of bad news",
            "first pass: done, second pass=2",
            "sklearn-df_train_features_Normalized2_more",
            "prod-AKIAIOSFODNN7EXAMPLE-x-long",
            "à la carte: é=ü, 日本語: 値",
            "",
        ] {
            assert!(
                matches!(scrub(text), Cow::Borrowed(_)),
                "{text}: {}",
                scrub(text)
            );
        }
    }

    #[test]
    fn never_panics_and_settles() {
        for text in [
            "=",
            ":",
            "password=",
            "password:",
            "password=\"",
            "password: '",
            "://",
            "a://@",
            "a://:@b",
            "é=",
            "é: é",
            "bearer ",
            "token=é",
            "\"token\": \"é",
            "ghp_",
            "=password",
            "::::",
            "====",
        ] {
            let once = scrub(text).into_owned();
            assert_eq!(scrub(&once), once, "{text}");
        }
    }

    #[test]
    fn takes_time_in_proportion_to_the_text() {
        for repeated in [
            "password=",
            "a:",
            "x://",
            "bearer ",
            "eyJ.",
            "=",
            "ghp_",
            "a-",
            "ghp-",
            "hf_-",
            "x.",
            "'pass':",
            "]=",
        ] {
            let text = repeated.repeat(200_000);
            let started = std::time::Instant::now();
            let _ = scrub(&text);
            assert!(started.elapsed().as_secs() < 5, "{repeated}");
        }
    }
}
