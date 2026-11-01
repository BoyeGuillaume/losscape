//! Hides values that look like secrets in what a run records: string values under names
//! like `api_key` or `HF_TOKEN` (in the config and captured environment variables), and
//! strings shaped like well-known credentials anywhere in the config.

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
];

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

/// Hides the value of an environment variable or argument called `name` if needed.
pub fn redact_str(name: &str, value: &str) -> String {
    if (secret_name(name) && !value.is_empty()) || looks_like_token(value) {
        HIDDEN.to_owned()
    } else {
        value.to_owned()
    }
}

/// Hides secrets in a JSON object, recursively. Numbers and booleans under secret names
/// are kept (`key_dim: 64` is a hyperparameter, not a secret).
pub fn redact_json(map: &mut Map<String, Value>) {
    for (name, value) in map.iter_mut() {
        redact_value(Some(name), value);
    }
}

fn redact_value(name: Option<&str>, value: &mut Value) {
    match value {
        Value::String(s) => {
            if (name.is_some_and(secret_name) && !s.is_empty()) || looks_like_token(s) {
                *s = HIDDEN.to_owned();
            }
        }
        Value::Object(map) => redact_json(map),
        // Lists inherit the name: `api_keys: [...]`.
        Value::Array(items) => items.iter_mut().for_each(|v| redact_value(name, v)),
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
}
