//! Secret material: random tokens, API keys, password hashing.
//!
//! All randomness comes from the operating system CSPRNG (`getrandom`), never from a
//! userspace PRNG.

use std::sync::LazyLock;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::error::ApiError;

/// Prefix of API keys, so secret scanners (and people) recognize leaked ones.
pub const API_KEY_PREFIX: &str = "df_";
/// Characters of an API key shown in the UI to tell keys apart.
const API_KEY_DISPLAY_LEN: usize = 10;
/// Upper bound on accepted passwords, so hashing can't be used for denial of service.
pub const MAX_PASSWORD_LEN: usize = 1024;

/// 256 random bits from the OS, base64url-encoded (43 characters).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random number generator unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// SHA-256, for storing high-entropy secrets (session tokens, API keys, OAuth state).
/// Those can't be guessed, so a fast hash is the right tool; passwords use Argon2id.
pub fn sha256(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

pub struct NewApiKey {
    /// Shown to the user once, never stored.
    pub secret: String,
    pub prefix: String,
    pub hash: Vec<u8>,
}

pub fn new_api_key() -> NewApiKey {
    let secret = format!("{API_KEY_PREFIX}{}", random_token());
    NewApiKey {
        prefix: secret[..API_KEY_DISPLAY_LEN].to_owned(),
        hash: sha256(&secret),
        secret,
    }
}

/// Argon2id with the library defaults: version 0x13, m = 19 MiB, t = 2, p = 1 (OWASP's
/// recommended minimum), 16-byte salt from the OS. The PHC string records algorithm,
/// parameters and salt, so parameters can be raised later without breaking old hashes.
fn argon2() -> Argon2<'static> {
    Argon2::default()
}

/// Argon2 takes 19 MiB and a CPU for each hash, on purpose. A few at a time, so that a flood
/// of sign-ins, which anyone can send, slows down sign-ins only.
static HASHING: Semaphore = Semaphore::const_new(8);
const HASHING_WAIT: Duration = Duration::from_secs(10);

async fn hashing_slot() -> Result<SemaphorePermit<'static>, ApiError> {
    tokio::time::timeout(HASHING_WAIT, HASHING.acquire())
        .await
        .map_err(|_| ApiError::busy())
        .map(|slot| slot.expect("never closed"))
}

/// Hashes on a blocking thread: Argon2 is deliberately slow and memory-hard.
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    let _slot = hashing_slot().await?;
    tokio::task::spawn_blocking(move || {
        argon2()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|err| {
                tracing::error!(%err, "password hashing failed");
                ApiError::internal()
            })
    })
    .await
    .map_err(|_| ApiError::internal())?
}

/// Verifies against `hash`, or against a fixed dummy hash when the user doesn't exist,
/// so a login takes the same time whether or not the username is known.
pub async fn verify_password(password: String, hash: Option<String>) -> Result<bool, ApiError> {
    let _slot = hashing_slot().await?;
    static DUMMY: LazyLock<String> = LazyLock::new(|| {
        argon2()
            .hash_password(b"losscape-dummy-password")
            .expect("hashing a constant")
            .to_string()
    });
    Ok(tokio::task::spawn_blocking(move || {
        let known = hash.is_some();
        let hash = hash.unwrap_or_else(|| DUMMY.clone());
        let matches = argon2()
            .verify_password(password.as_bytes(), hash.as_str())
            .is_ok();
        known && matches
    })
    .await
    .unwrap_or(false))
}

/// PKCE (RFC 7636) S256 challenge for a verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_url_safe() {
        let (a, b) = (random_token(), random_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn api_keys_store_only_a_hash() {
        let key = new_api_key();
        assert!(key.secret.starts_with("df_") && key.secret.len() == 46);
        assert!(key.secret.starts_with(&key.prefix));
        assert_eq!(key.hash, sha256(&key.secret));
        assert_ne!(new_api_key().hash, key.hash);
    }

    #[tokio::test]
    async fn passwords_use_salted_argon2id() {
        let a = hash_password("correct horse battery".into()).await.unwrap();
        let b = hash_password("correct horse battery".into()).await.unwrap();
        assert!(a.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{a}");
        assert_ne!(a, b, "fresh salt per hash");
        assert!(
            verify_password("correct horse battery".into(), Some(a.clone()))
                .await
                .unwrap()
        );
        assert!(!verify_password("wrong".into(), Some(a)).await.unwrap());
        assert!(!verify_password("anything".into(), None).await.unwrap());
    }

    #[test]
    fn pkce_matches_the_oauth2_crate() {
        // GitHub sign-in builds the challenge by hand; OIDC uses the oauth2 crate's.
        for _ in 0..5 {
            let verifier = random_token();
            let reference = openidconnect::PkceCodeChallenge::from_code_verifier_sha256(
                &openidconnect::PkceCodeVerifier::new(verifier.clone()),
            );
            assert_eq!(pkce_challenge(&verifier), reference.as_str());
        }
    }
}
