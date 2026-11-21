//! Saved passwords (servers', shares', encrypted drives'): each sealed with
//! AES-256-GCM in the settings folder's `passwords.json`, so they go along
//! when the folder is copied. The key that opens them stays in this
//! computer's keyring, never in the folder: made from the master password
//! (PBKDF2), without which nothing is saved, so another computer opens them
//! once it is typed there. The keyring keeps the key, not the master password,
//! which may be in use elsewhere too. Calls may wait on the keyring (KWallet
//! asks to open the wallet): not on the UI thread.

use std::collections::BTreeMap;
use std::io;
use std::num::NonZeroU32;
use std::sync::Mutex;

use keyring::{Entry, Error};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "Spark Commander";
/// The keyring entry of the key.
const KEY_ENTRY: &str = "passwords-key";
const FILE: &str = "passwords.json";
/// PBKDF2-HMAC-SHA256's rounds for a master password, as OWASP advises.
const ROUNDS: u32 = 600_000;
/// Sealed in the file, it tells a key the folder's own.
const CHECK: &str = "check";

type Key = [u8; 32];

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Vault {
    /// The master password's salt (hex) and rounds, when the key is made from one.
    master: Option<(String, u32)>,
    /// `CHECK` sealed under the key; empty before the first password.
    check: String,
    /// Each password sealed, by what it is for (`sftp://user@host:22`).
    passwords: BTreeMap<String, String>,
}

impl Vault {
    fn opens_with(&self, key: &Key) -> bool {
        open(key, CHECK, &self.check).is_some()
    }

    /// Every password sealed again under `new`, the check too.
    fn reseal(&mut self, old: &Key, new: &Key) -> io::Result<()> {
        for (what, sealed) in &mut self.passwords {
            let plain = open(old, what, sealed).ok_or_else(|| io::Error::other(crate::t!("passwords.error.not_opened", what = what)))?;
            *sealed = seal(new, what, &plain)?;
        }
        self.check = seal(new, CHECK, CHECK.as_bytes())?;
        Ok(())
    }
}

/// This run's key, once read from the keyring or made from the master password.
static KEY: Mutex<Option<Key>> = Mutex::new(None);
/// One change of the file at a time.
static CHANGING: Mutex<()> = Mutex::new(());

/// The password kept for `what` (`sftp://user@host:22`), if any. One an
/// earlier version kept in the keyring moves into the folder.
pub fn password(what: &str) -> Option<String> {
    let vault = load().ok()?;
    if let Some(sealed) = vault.passwords.get(what) {
        let key = key_for(&vault).ok()??;
        return open(&key, what, sealed).and_then(|plain| String::from_utf8(plain).ok());
    }
    let old = Entry::new(SERVICE, what).ok()?.get_password().ok()?;
    if remember(what, &old).is_ok() {
        let _ = forget_old(what);
    }
    Some(old)
}

pub fn remember(what: &str, password: &str) -> io::Result<()> {
    let _one = CHANGING.lock().unwrap();
    let mut vault = load()?;
    let key = ready(&vault)?;
    vault.passwords.insert(what.to_owned(), seal(&key, what, password.as_bytes())?);
    save(&vault)
}

pub fn forget(what: &str) -> io::Result<()> {
    {
        let _one = CHANGING.lock().unwrap();
        let mut vault = load()?;
        if vault.passwords.remove(what).is_some() {
            save(&vault)?;
        }
    }
    forget_old(what)
}

/// How many passwords are saved.
pub fn saved() -> usize {
    load().map_or(0, |vault| vault.passwords.len())
}

/// Whether a password is saved for `what`, without opening any.
pub fn is_saved(what: &str) -> bool {
    load().is_ok_and(|vault| vault.passwords.contains_key(what))
}

/// Whether a master password makes the key.
pub fn has_master() -> bool {
    load().is_ok_and(|vault| vault.master.is_some())
}

/// The saved passwords wait for the master password here: the folder came
/// from another computer, or this one keeps no keyring.
pub fn locked() -> bool {
    load().is_ok_and(|vault| vault.master.is_some() && !vault.passwords.is_empty() && !matches!(key_for(&vault), Ok(Some(_))))
}

/// Opens the saved passwords with the master password; this computer's
/// keyring keeps the key from then on (without one, this run does).
pub fn unlock(master: &str) -> io::Result<()> {
    let vault = load()?;
    let (salt, rounds) = vault.master.clone().ok_or_else(|| io::Error::other(crate::t!("passwords.error.no_master")))?;
    let salt = unhex(&salt).ok_or_else(|| io::Error::other(crate::t!("passwords.error.salt_damaged")))?;
    let key = derive(master, &salt, rounds);
    if !vault.opens_with(&key) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, crate::t!("passwords.error.wrong")));
    }
    *KEY.lock().unwrap() = Some(key);
    let _ = keep(&key);
    Ok(())
}

/// Sets or changes the master password; every saved password is sealed
/// again under the new key. This computer's keyring keeps it; without one,
/// the master password opens them each run.
pub fn set_master(master: &str) -> io::Result<()> {
    let _one = CHANGING.lock().unwrap();
    let mut vault = load()?;
    // With nothing saved, no key is needed yet: none is asked of the keyring.
    let old = match vault.passwords.is_empty() {
        true => None,
        false => match key_for(&vault)? {
            None if vault.master.is_some() => return Err(locked_error()),
            // An earlier version's random key, gone from the keyring: they cannot be opened.
            None => {
                vault.passwords.clear();
                None
            }
            key => key,
        },
    };
    let salt: [u8; 16] = random()?;
    let new = derive(master, &salt, ROUNDS);
    vault.reseal(&old.unwrap_or(new), &new)?;
    vault.master = Some((hex(&salt), ROUNDS));
    save(&vault)?;
    *KEY.lock().unwrap() = Some(new);
    let _ = keep(&new);
    Ok(())
}

/// The key a server's password is kept under.
pub fn key(scheme: &str, user: &str, host: &str, port: u16) -> String {
    format!("{scheme}://{user}@{host}:{port}")
}

fn load() -> io::Result<Vault> {
    crate::settings::read_json(FILE)
}

fn save(vault: &Vault) -> io::Result<()> {
    crate::settings::write_json(FILE, vault)
}

/// The key that opens `vault`: this run's, else this computer's; none when
/// neither does (none kept yet, or the folder came from elsewhere). A
/// keyring that cannot be read fails, so that no new key replaces one only
/// out of reach.
fn key_for(vault: &Vault) -> io::Result<Option<Key>> {
    let fits = |key: &Key| vault.check.is_empty() || vault.opens_with(key);
    let mut held = KEY.lock().unwrap();
    if !held.as_ref().is_some_and(fits) {
        *held = kept()?.filter(fits);
    }
    Ok(*held)
}

/// The key to seal with: the master password's, once it is set and open here.
fn ready(vault: &Vault) -> io::Result<Key> {
    if vault.master.is_none() {
        return Err(io::Error::other(crate::t!("passwords.error.no_master")));
    }
    key_for(vault)?.ok_or_else(locked_error)
}

fn locked_error() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, crate::t!("passwords.error.locked"))
}

fn kept() -> io::Result<Option<Key>> {
    match Entry::new(SERVICE, KEY_ENTRY).map_err(io::Error::other)?.get_password() {
        Ok(text) => Ok(unhex(&text).and_then(|bytes| bytes.try_into().ok())),
        Err(Error::NoEntry) => Ok(None),
        Err(err) => Err(io::Error::other(err)),
    }
}

fn keep(key: &Key) -> io::Result<()> {
    Entry::new(SERVICE, KEY_ENTRY).and_then(|entry| entry.set_password(&hex(key))).map_err(io::Error::other)
}

/// An earlier version's keyring entry for `what`, gone.
fn forget_old(what: &str) -> io::Result<()> {
    match Entry::new(SERVICE, what).map_err(io::Error::other)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(err) => Err(io::Error::other(err)),
    }
}

fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    SystemRandom::new().fill(&mut bytes).map_err(|_| io::Error::other(crate::t!("passwords.error.no_random")))?;
    Ok(bytes)
}

fn derive(master: &str, salt: &[u8], rounds: u32) -> Key {
    let mut key = [0; 32];
    let rounds = NonZeroU32::new(rounds).unwrap_or(NonZeroU32::MIN);
    ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256, rounds, salt, master.as_bytes(), &mut key);
    key
}

/// `plain` sealed under `key`, bound to `what` (a password moved to another
/// entry does not open): the nonce, then the sealed bytes, in hex.
fn seal(key: &Key, what: &str, plain: &[u8]) -> io::Result<String> {
    let failed = |_| io::Error::other(crate::t!("passwords.error.not_sealed"));

    let nonce: [u8; NONCE_LEN] = random()?;
    let sealer = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(failed)?);
    let mut sealed = plain.to_vec();
    sealer.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(what.as_bytes()), &mut sealed).map_err(failed)?;
    Ok(hex(&[&nonce[..], &sealed].concat()))
}

fn open(key: &Key, what: &str, sealed: &str) -> Option<Vec<u8>> {
    let bytes = unhex(sealed)?;
    let (nonce, sealed) = bytes.split_at_checked(NONCE_LEN)?;
    let opener = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut sealed = sealed.to_vec();
    let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
    opener.open_in_place(nonce, Aad::from(what.as_bytes()), &mut sealed).ok().map(|plain| plain.to_vec())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_under_a_key_and_for_one_entry() {
        let (key, other): (Key, Key) = (random().unwrap(), random().unwrap());
        let sealed = seal(&key, "sftp://me@host:22", b"s3cret").unwrap();
        assert_eq!(open(&key, "sftp://me@host:22", &sealed).unwrap(), b"s3cret");
        assert_eq!(open(&other, "sftp://me@host:22", &sealed), None, "another key");
        assert_eq!(open(&key, "sftp://you@host:22", &sealed), None, "moved to another entry");
        assert_ne!(seal(&key, "sftp://me@host:22", b"s3cret").unwrap(), sealed, "a fresh nonce each time");
    }

    #[test]
    fn nothing_is_saved_without_a_master_password() {
        assert!(ready(&Vault::default()).is_err());
    }

    #[test]
    fn a_master_password_seals_them_again() {
        let random_key: Key = random().unwrap();
        let mut vault = Vault::default();
        vault.passwords.insert("ftp://a@b:21".into(), seal(&random_key, "ftp://a@b:21", b"pw").unwrap());
        vault.check = seal(&random_key, CHECK, CHECK.as_bytes()).unwrap();
        // Few rounds: the test's speed, not the strength.
        let salt: [u8; 16] = random().unwrap();
        let master = derive("correct horse", &salt, 1000);
        vault.reseal(&random_key, &master).unwrap();
        assert!(vault.opens_with(&master) && !vault.opens_with(&random_key));
        assert!(!vault.opens_with(&derive("wrong horse", &salt, 1000)));
        assert_eq!(derive("correct horse", &salt, 1000), master, "the same key on another computer");
        assert_eq!(open(&master, "ftp://a@b:21", &vault.passwords["ftp://a@b:21"]).unwrap(), b"pw");
    }
}
