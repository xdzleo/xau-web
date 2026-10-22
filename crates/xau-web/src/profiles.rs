//! One isolated browser profile per account, for the native windows.
//!
//! The same idea as the Android app's WebView profiles: an account's
//! Microsoft pages (sign-in, the Store checkout) keep their cookies, cache
//! and storage in a folder of their own, so one account's session never
//! reaches another's, and removing an account deletes its folder.
//!
//! A sign-in runs in a fresh `signin-…` folder that becomes the account's
//! when the sign-in succeeds; leftovers of cancelled ones are swept at the
//! next sign-in.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn root() -> PathBuf {
    xau_mobile::accounts::data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("webprofiles")
}

fn map_path() -> PathBuf {
    root().join("profiles.json")
}

fn load() -> BTreeMap<String, String> {
    std::fs::read(map_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(map: &BTreeMap<String, String>) {
    let _ = std::fs::create_dir_all(root());
    if let Ok(b) = serde_json::to_vec_pretty(map) {
        let _ = std::fs::write(map_path(), b);
    }
}

fn safe(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A new, empty profile for a sign-in in progress, after sweeping the
/// ones no account kept.
pub fn new_signin() -> PathBuf {
    sweep();
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
    root().join(format!("signin-{id}"))
}

/// The sign-in in `dir` succeeded for `xuid`: it becomes that account's
/// profile. Signing in again as the same account replaces (and deletes)
/// the profile it had.
pub fn bind(xuid: &str, dir: &std::path::Path) {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { return };
    if !safe(name) {
        return;
    }
    let mut map = load();
    if let Some(old) = map.insert(xuid.to_string(), name.to_string()) {
        if old != name && safe(&old) {
            let _ = std::fs::remove_dir_all(root().join(old));
        }
    }
    save(&map);
}

/// The profile an account's pages use: its own, or a new one that the
/// account keeps from now on (an account signed in before this existed).
pub fn of(xuid: &str) -> PathBuf {
    let mut map = load();
    if let Some(n) = map.get(xuid).filter(|n| safe(n)) {
        return root().join(n);
    }
    let dir = new_signin();
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    map.insert(xuid.to_string(), name);
    save(&map);
    dir
}

/// The account is gone: delete everything its pages stored.
pub fn forget(xuid: &str) {
    let mut map = load();
    if let Some(name) = map.remove(xuid) {
        if safe(&name) {
            let _ = std::fs::remove_dir_all(root().join(name));
        }
        save(&map);
    }
}

/// Sign-in profiles no account kept (cancelled or failed sign-ins).
fn sweep() {
    let kept: Vec<String> = load().into_values().collect();
    let Ok(entries) = std::fs::read_dir(root()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("signin-") && !kept.contains(&name) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}
