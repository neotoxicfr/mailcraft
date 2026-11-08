//! Secrets: one random master key kept in the OS credential vault (Windows Credential Manager).
//! It unlocks the encrypted database and seals refresh tokens (AES-256-GCM), so no secret is
//! ever written in clear, even when the user keeps the mail cache unencrypted.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};

const SERVICE: &str = "MailCraft";
const USER: &str = "master-key";
const NONCE_LEN: usize = 12;

#[derive(Clone)]
pub struct MasterKey([u8; 32]);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

#[derive(Debug)]
pub enum VaultError {
    Os(String),
    Random,
    Corrupt,
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Os(e) => write!(f, "credential vault: {e}"),
            Self::Random => f.write_str("no random numbers from the system"),
            Self::Corrupt => f.write_str("sealed secret is damaged or was sealed with another key"),
        }
    }
}

impl std::error::Error for VaultError {}

impl MasterKey {
    pub fn random() -> Result<Self, VaultError> {
        let mut k = [0u8; 32];
        getrandom::fill(&mut k).map_err(|_| VaultError::Random)?;
        Ok(Self(k))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Reads the key from the OS vault, creating and storing it on first use.
    pub fn load_or_create() -> Result<Self, VaultError> {
        let entry = os_entry(USER)?;
        match entry.get_secret() {
            Ok(bytes) => bytes.try_into().map(Self).map_err(|_| VaultError::Corrupt),
            Err(keyring_core::Error::NoEntry) => {
                let key = Self::random()?;
                entry
                    .set_secret(&key.0)
                    .map_err(|e| VaultError::Os(e.to_string()))?;
                Ok(key)
            }
            Err(e) => Err(VaultError::Os(e.to_string())),
        }
    }

    /// SQLCipher raw key literal: the key is used as is, without passphrase derivation.
    pub fn sqlcipher_pragma(&self) -> String {
        let hex: String = self.0.iter().map(|b| format!("{b:02x}")).collect();
        format!("x'{hex}'")
    }

    /// `nonce || ciphertext+tag`.
    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| VaultError::Random)?;
        let sealed = self
            .cipher()
            .encrypt(&Nonce::from(nonce), plain)
            .map_err(|_| VaultError::Corrupt)?;
        let mut out = nonce.to_vec();
        out.extend(sealed);
        Ok(out)
    }

    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, VaultError> {
        if sealed.len() < NONCE_LEN {
            return Err(VaultError::Corrupt);
        }
        let (nonce, body) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| VaultError::Corrupt)?;
        self.cipher()
            .decrypt(&Nonce::from(nonce), body)
            .map_err(|_| VaultError::Corrupt)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.0))
    }
}

fn os_entry(user: &str) -> Result<keyring_core::Entry, VaultError> {
    #[cfg(windows)]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        let mut failed = None;
        INIT.call_once(|| match windows_native_keyring_store::Store::new() {
            Ok(store) => keyring_core::set_default_store(store),
            Err(e) => failed = Some(e.to_string()),
        });
        if let Some(e) = failed {
            return Err(VaultError::Os(e));
        }
    }
    keyring_core::Entry::new(SERVICE, user).map_err(|e| VaultError::Os(e.to_string()))
}

/// A named secret in the OS vault (the Claude API key), `None` when not set.
pub fn secret(name: &str) -> Result<Option<String>, VaultError> {
    match os_entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(VaultError::Os(e.to_string())),
    }
}

pub fn set_secret(name: &str, value: &str) -> Result<(), VaultError> {
    os_entry(name)?
        .set_password(value)
        .map_err(|e| VaultError::Os(e.to_string()))
}

pub fn delete_secret(name: &str) -> Result<(), VaultError> {
    match os_entry(name)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(VaultError::Os(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::MasterKey;

    #[test]
    fn seal_round_trip_and_tamper() {
        let key = MasterKey::random().unwrap();
        let sealed = key.seal(b"refresh-token").unwrap();
        assert_ne!(&sealed[12..], b"refresh-token");
        assert_eq!(key.open(&sealed).unwrap(), b"refresh-token");

        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(key.open(&bad).is_err());
        assert!(MasterKey::random().unwrap().open(&sealed).is_err());
        assert!(key.open(&[1, 2, 3]).is_err());
    }
}
