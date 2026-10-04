//! The encrypted vault that protects saved passwords and private keys.
//!
//! Each user has one random data key ([`VaultKey`]). Secrets are encrypted
//! with it, and the database only ever holds that key wrapped by a key
//! derived from the user's passphrase. So a stolen database file reveals no
//! secrets, and changing the passphrase means re-wrapping one key rather than
//! re-encrypting everything.
//!
//! ```text
//! passphrase --Argon2id(salt)--> master --+--> verifier  (stored; proves the passphrase at login)
//!                                         +--> wrapping key --XChaCha20-Poly1305--> wrapped data key (stored)
//! ```

use argon2::Argon2;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    // The OS random source failing is not something we can recover from.
    getrandom::fill(&mut bytes).expect("operating system random source");
    bytes
}

pub fn new_salt() -> [u8; SALT_LEN] {
    random()
}

/// What a passphrase turns into. Deriving is deliberately slow (tens of
/// milliseconds and ~19 MB of memory), so call it from a blocking thread.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Derived {
    /// Safe to store: it cannot be turned back into the wrapping key.
    verifier: [u8; KEY_LEN],
    wrapping_key: [u8; KEY_LEN],
}

impl Derived {
    pub fn from_passphrase(passphrase: &str, salt: &[u8]) -> Self {
        let mut master = [0u8; KEY_LEN];
        Argon2::default()
            .hash_password_into(passphrase.as_bytes(), salt, &mut master)
            .expect("argon2 with default parameters and a 32-byte output");
        let derived = Self {
            verifier: subkey(&master, b"fastssh login verifier"),
            wrapping_key: subkey(&master, b"fastssh vault wrapping key"),
        };
        master.zeroize();
        derived
    }

    pub fn verifier(&self) -> &[u8; KEY_LEN] {
        &self.verifier
    }

    /// Compares in constant time, so response timing leaks nothing.
    pub fn verifier_matches(&self, stored: &[u8]) -> bool {
        stored.len() == KEY_LEN
            && self
                .verifier
                .iter()
                .zip(stored)
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

fn subkey(master: &[u8; KEY_LEN], purpose: &[u8]) -> [u8; KEY_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(purpose);
    hasher.update(master);
    hasher.finalize().into()
}

/// A user's data key. It exists only in memory, and only while that user has
/// an unlocked session.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct VaultKey([u8; KEY_LEN]);

impl VaultKey {
    pub fn generate() -> Self {
        Self(random())
    }

    /// Encrypts this key for storage under a passphrase.
    pub fn wrap(&self, derived: &Derived) -> Vec<u8> {
        seal(&derived.wrapping_key, b"fastssh vault key", &self.0)
    }

    /// `None` means the passphrase was wrong (or the data was tampered with).
    pub fn unwrap(wrapped: &[u8], derived: &Derived) -> Option<Self> {
        let mut plain = open(&derived.wrapping_key, b"fastssh vault key", wrapped)?;
        let key = <[u8; KEY_LEN]>::try_from(plain.as_slice()).ok().map(Self);
        plain.zeroize();
        key
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        seal(&self.0, b"fastssh secret", plaintext)
    }

    pub fn decrypt(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        open(&self.0, b"fastssh secret", sealed)
    }
}

/// Output layout: 24-byte random nonce, then ciphertext with its auth tag.
/// `purpose` is authenticated, so a blob cannot be reused in another role.
fn seal(key: &[u8; KEY_LEN], purpose: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let nonce: [u8; NONCE_LEN] = random();
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: plaintext, aad: purpose })
        .expect("encrypting an in-memory buffer");
    [nonce.as_slice(), &ciphertext].concat()
}

fn open(key: &[u8; KEY_LEN], purpose: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let (nonce, ciphertext) = sealed.split_at_checked(NONCE_LEN)?;
    let nonce = <[u8; NONCE_LEN]>::try_from(nonce).ok()?;
    XChaCha20Poly1305::new(key.into())
        .decrypt(&XNonce::from(nonce), Payload { msg: ciphertext, aad: purpose })
        .ok()
}

/// SHA-256, used to store session tokens without storing the tokens themselves.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A fresh unguessable token, as hex.
pub fn new_token() -> String {
    hex::encode(random::<32>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_passphrase_unlocks_wrong_one_does_not() {
        let salt = new_salt();
        let right = Derived::from_passphrase("correct horse", &salt);
        let key = VaultKey::generate();
        let wrapped = key.wrap(&right);

        let again = Derived::from_passphrase("correct horse", &salt);
        assert!(again.verifier_matches(right.verifier()));
        let unlocked = VaultKey::unwrap(&wrapped, &again).expect("unlocks");
        assert_eq!(unlocked.0, key.0);

        let wrong = Derived::from_passphrase("battery staple", &salt);
        assert!(!wrong.verifier_matches(right.verifier()));
        assert!(VaultKey::unwrap(&wrapped, &wrong).is_none());
    }

    #[test]
    fn secrets_round_trip_and_reject_tampering() {
        let key = VaultKey::generate();
        let sealed = key.encrypt(b"hunter2");
        assert!(!sealed.windows(7).any(|w| w == b"hunter2"));
        assert_eq!(key.decrypt(&sealed).as_deref(), Some(b"hunter2".as_slice()));

        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(key.decrypt(&tampered).is_none());
        assert!(VaultKey::generate().decrypt(&sealed).is_none());
        assert!(key.decrypt(b"short").is_none());
    }

    #[test]
    fn a_wrapped_key_cannot_pass_as_a_secret() {
        let derived = Derived::from_passphrase("p", &new_salt());
        let key = VaultKey(derived.wrapping_key);
        let wrapped = VaultKey::generate().wrap(&derived);
        assert!(key.decrypt(&wrapped).is_none());
    }
}
