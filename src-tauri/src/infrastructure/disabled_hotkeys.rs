//! System `Win+<letter>` hotkey liberation through the Windows
//! `DisabledHotkeys` registry value.
//!
//! Windows lets each user disable the shell's built-in `Win+<letter>` hotkeys
//! (for example `Win+V` for clipboard history) by listing the letters in
//! `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced`
//! under the `DisabledHotkeys` string value. Once the shell stops claiming a
//! combination, OpenDeskTools can take it over with a standard `RegisterHotKey`
//! registration instead of relying on the low-level keyboard hook fallback.
//!
//! This module keeps that registry value in sync with the hotkey configuration:
//! it only ever adds or removes the letters OpenDeskTools itself introduced, so
//! any letters the user disabled manually stay untouched. The change requires an
//! Explorer restart (or sign-out / reboot) to take effect, and the letters are
//! removed again when the binding is cleared or the application exits.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use thiserror::Error;

use super::hotkey::{HotkeyBindingClassification, HotkeySnapshot};
use super::storage::{StorageError, StorageService};

/// Settings key under which the letters OpenDeskTools currently owns in the
/// `DisabledHotkeys` value are persisted.
const OWNED_LETTERS_SETTING_KEY: &str = "hotkeys.disabled_hotkeys.owned.v1";

#[cfg(windows)]
const ADVANCED_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
#[cfg(windows)]
const DISABLED_HOTKEYS_VALUE: &str = "DisabledHotkeys";

#[derive(Debug, Error)]
pub enum DisabledHotkeysError {
    #[error("disabled-hotkeys state lock is poisoned")]
    LockPoisoned,
    #[error("failed to access owned-letters storage: {0}")]
    Storage(#[from] StorageError),
    #[cfg(windows)]
    #[error("Windows registry operation {operation} failed with status {status}")]
    Registry {
        operation: &'static str,
        status: u32,
    },
}

/// Outcome of a single reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisabledHotkeysOutcome {
    /// Whether the registry value was rewritten during this pass.
    pub changed: bool,
    /// The letters OpenDeskTools now owns in the registry value.
    pub managed_letters: Vec<char>,
    /// The full registry value after reconciliation.
    pub registry_value: String,
}

/// Abstraction over the `DisabledHotkeys` registry string so the coordinator
/// can be exercised without touching the real registry.
pub trait DisabledHotkeysRegistry: Send + Sync {
    fn read(&self) -> Result<String, DisabledHotkeysError>;
    fn write(&self, value: &str) -> Result<(), DisabledHotkeysError>;
    fn clear(&self) -> Result<(), DisabledHotkeysError>;
}

/// Abstraction over persistence of the letters OpenDeskTools owns.
pub trait OwnedLettersStore: Send + Sync {
    fn read_owned(&self) -> Result<String, DisabledHotkeysError>;
    fn write_owned(&self, letters: &str) -> Result<(), DisabledHotkeysError>;
}

impl OwnedLettersStore for StorageService {
    fn read_owned(&self) -> Result<String, DisabledHotkeysError> {
        Ok(self
            .read_setting(OWNED_LETTERS_SETTING_KEY)?
            .unwrap_or_default())
    }

    fn write_owned(&self, letters: &str) -> Result<(), DisabledHotkeysError> {
        self.write_settings(&[(OWNED_LETTERS_SETTING_KEY, letters)])?;
        Ok(())
    }
}

/// Coordinates the registry value with the desired set of disabled letters.
pub struct SystemHotkeyDisabler {
    registry: Arc<dyn DisabledHotkeysRegistry>,
    owned: Arc<dyn OwnedLettersStore>,
    lock: Mutex<()>,
}

impl std::fmt::Debug for SystemHotkeyDisabler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SystemHotkeyDisabler").finish()
    }
}

impl SystemHotkeyDisabler {
    pub fn new(
        registry: Arc<dyn DisabledHotkeysRegistry>,
        owned: Arc<dyn OwnedLettersStore>,
    ) -> Self {
        Self {
            registry,
            owned,
            lock: Mutex::new(()),
        }
    }

    /// Builds the coordinator that talks to the real system registry on Windows
    /// and to a harmless no-op registry elsewhere.
    pub fn for_system(owned: Arc<dyn OwnedLettersStore>) -> Self {
        #[cfg(windows)]
        let registry: Arc<dyn DisabledHotkeysRegistry> = Arc::new(SystemDisabledHotkeysRegistry);
        #[cfg(not(windows))]
        let registry: Arc<dyn DisabledHotkeysRegistry> = Arc::new(NoopDisabledHotkeysRegistry);
        Self::new(registry, owned)
    }

    /// Ensures the registry value reflects exactly the `desired` letters that
    /// OpenDeskTools should own, while preserving letters owned by the user.
    pub fn reconcile(
        &self,
        desired: &BTreeSet<char>,
    ) -> Result<DisabledHotkeysOutcome, DisabledHotkeysError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| DisabledHotkeysError::LockPoisoned)?;

        let current_raw = self.registry.read()?;
        let current = normalize_letters(&current_raw);
        let current_value: String = current.iter().collect();
        let owned = parse_letters(&self.owned.read_owned()?);
        let owned_value: String = owned.iter().collect();

        let (next_chars, next_owned) = reconcile_letters(&current, &owned, desired);
        let next_value: String = next_chars.iter().collect();

        let changed = next_value != current_value;
        if changed {
            if next_value.is_empty() {
                self.registry.clear()?;
            } else {
                self.registry.write(&next_value)?;
            }
        }

        let next_owned_value: String = next_owned.iter().collect();
        if next_owned_value != owned_value {
            self.owned.write_owned(&next_owned_value)?;
        }

        Ok(DisabledHotkeysOutcome {
            changed,
            managed_letters: next_owned.iter().copied().collect(),
            registry_value: next_value,
        })
    }

    /// Removes every letter OpenDeskTools introduced, restoring the system
    /// defaults for the letters it no longer needs.
    pub fn restore_all(&self) -> Result<DisabledHotkeysOutcome, DisabledHotkeysError> {
        self.reconcile(&BTreeSet::new())
    }
}

/// Extracts the set of `Win+<letter>` combinations OpenDeskTools should disable
/// at the system level based on the current hotkey configuration.
pub fn desired_disabled_letters(snapshot: &HotkeySnapshot) -> BTreeSet<char> {
    snapshot
        .actions
        .iter()
        .filter(|action| {
            action.configured_enabled
                && action.action_available
                && action.force_override_system
                && action.classification == HotkeyBindingClassification::SystemReserved
        })
        .filter_map(|action| win_single_letter(&action.binding))
        .collect()
}

/// Returns the uppercase letter of a bare `Win+<letter>` binding, or `None` for
/// any other combination (the `DisabledHotkeys` value only affects `Win+<key>`).
pub fn win_single_letter(binding: &str) -> Option<char> {
    let mut parts = binding.split('+');
    let modifier = parts.next()?;
    let key = parts.next()?;
    if parts.next().is_some() || !modifier.eq_ignore_ascii_case("Win") {
        return None;
    }
    let mut key_chars = key.chars();
    let letter = key_chars.next()?;
    if key_chars.next().is_some() || !letter.is_ascii_alphanumeric() {
        return None;
    }
    Some(letter.to_ascii_uppercase())
}

/// Uppercases, filters to alphanumerics, and de-duplicates while preserving the
/// original ordering of a raw registry value.
fn normalize_letters(raw: &str) -> Vec<char> {
    let mut seen = BTreeSet::new();
    raw.chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .map(|value| value.to_ascii_uppercase())
        .filter(|value| seen.insert(*value))
        .collect()
}

fn parse_letters(raw: &str) -> BTreeSet<char> {
    raw.chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .map(|value| value.to_ascii_uppercase())
        .collect()
}

/// Computes the next registry value and the next owned set.
///
/// - Letters OpenDeskTools previously owned but no longer wants are removed.
/// - Desired letters not present yet are appended and become owned.
/// - Letters already present that OpenDeskTools did not introduce (user-managed)
///   are preserved and never claimed, so they survive later unbinding.
fn reconcile_letters(
    current: &[char],
    owned: &BTreeSet<char>,
    desired: &BTreeSet<char>,
) -> (Vec<char>, BTreeSet<char>) {
    let to_remove: BTreeSet<char> = owned.difference(desired).copied().collect();
    let current_set: BTreeSet<char> = current.iter().copied().collect();

    let mut next: Vec<char> = current
        .iter()
        .copied()
        .filter(|value| !to_remove.contains(value))
        .collect();
    for letter in desired {
        if !next.contains(letter) {
            next.push(*letter);
        }
    }

    let mut next_owned: BTreeSet<char> = owned.intersection(desired).copied().collect();
    for letter in desired {
        if !current_set.contains(letter) {
            next_owned.insert(*letter);
        }
    }

    (next, next_owned)
}

/// No-op registry used on non-Windows platforms so the runtime can be built.
#[cfg(not(windows))]
struct NoopDisabledHotkeysRegistry;

#[cfg(not(windows))]
impl DisabledHotkeysRegistry for NoopDisabledHotkeysRegistry {
    fn read(&self) -> Result<String, DisabledHotkeysError> {
        Ok(String::new())
    }

    fn write(&self, _value: &str) -> Result<(), DisabledHotkeysError> {
        Ok(())
    }

    fn clear(&self) -> Result<(), DisabledHotkeysError> {
        Ok(())
    }
}

/// Registry implementation backed by the real Windows `DisabledHotkeys` value.
#[cfg(windows)]
struct SystemDisabledHotkeysRegistry;

#[cfg(windows)]
struct HkeyGuard(windows_sys::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for HkeyGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn registry_error(operation: &'static str, status: u32) -> DisabledHotkeysError {
    DisabledHotkeysError::Registry { operation, status }
}

#[cfg(windows)]
impl DisabledHotkeysRegistry for SystemDisabledHotkeysRegistry {
    fn read(&self) -> Result<String, DisabledHotkeysError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
        };

        let subkey = wide(ADVANCED_SUBKEY);
        let value_name = wide(DISABLED_HOTKEYS_VALUE);

        let mut key: HKEY = std::ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut key) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(String::new());
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);

        let mut value_type: u32 = 0;
        let mut byte_len: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                std::ptr::null_mut(),
                &mut byte_len,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(String::new());
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegQueryValueExW", status));
        }
        if value_type != REG_SZ || byte_len == 0 {
            return Ok(String::new());
        }

        let mut buffer: Vec<u16> = vec![0u16; (byte_len as usize).div_ceil(2)];
        let mut buffer_bytes: u32 = (buffer.len() * std::mem::size_of::<u16>()) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                buffer.as_mut_ptr() as *mut u8,
                &mut buffer_bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegQueryValueExW", status));
        }

        let units = (buffer_bytes as usize) / std::mem::size_of::<u16>();
        let slice = &buffer[..units.min(buffer.len())];
        let end = slice
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(slice.len());
        Ok(String::from_utf16_lossy(&slice[..end]))
    }

    fn write(&self, value: &str) -> Result<(), DisabledHotkeysError> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
        };

        let subkey = wide(ADVANCED_SUBKEY);
        let value_name = wide(DISABLED_HOTKEYS_VALUE);

        // The `Explorer\Advanced` key ships with every Windows profile, so a
        // plain open with write access is sufficient and avoids depending on the
        // security-attributes surface required by `RegCreateKeyExW`.
        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);

        let data = wide(value);
        let data_bytes = (data.len() * std::mem::size_of::<u16>()) as u32;
        let status = unsafe {
            RegSetValueExW(
                key.0,
                value_name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                data_bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegSetValueExW", status));
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), DisabledHotkeysError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{
            RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        };

        let subkey = wide(ADVANCED_SUBKEY);
        let value_name = wide(DISABLED_HOTKEYS_VALUE);

        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);

        let status = unsafe { RegDeleteValueW(key.0, value_name.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(registry_error("RegDeleteValueW", status))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::hotkey::{
        HotkeyActionId, HotkeyRegistrationSnapshot, HotkeyRuntimeState,
    };
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct FakeRegistry {
        value: StdMutex<Option<String>>,
        writes: StdMutex<Vec<String>>,
        clears: StdMutex<u32>,
    }

    impl FakeRegistry {
        fn with_value(value: &str) -> Self {
            let registry = FakeRegistry::default();
            *registry.value.lock().unwrap() = Some(value.to_owned());
            registry
        }

        fn snapshot(&self) -> Option<String> {
            self.value.lock().unwrap().clone()
        }

        fn write_count(&self) -> usize {
            self.writes.lock().unwrap().len()
        }

        fn clear_count(&self) -> u32 {
            *self.clears.lock().unwrap()
        }
    }

    impl DisabledHotkeysRegistry for FakeRegistry {
        fn read(&self) -> Result<String, DisabledHotkeysError> {
            Ok(self.value.lock().unwrap().clone().unwrap_or_default())
        }

        fn write(&self, value: &str) -> Result<(), DisabledHotkeysError> {
            *self.value.lock().unwrap() = Some(value.to_owned());
            self.writes.lock().unwrap().push(value.to_owned());
            Ok(())
        }

        fn clear(&self) -> Result<(), DisabledHotkeysError> {
            *self.value.lock().unwrap() = None;
            *self.clears.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeOwned {
        letters: StdMutex<String>,
    }

    impl FakeOwned {
        fn with(letters: &str) -> Self {
            let owned = FakeOwned::default();
            *owned.letters.lock().unwrap() = letters.to_owned();
            owned
        }

        fn value(&self) -> String {
            self.letters.lock().unwrap().clone()
        }
    }

    impl OwnedLettersStore for FakeOwned {
        fn read_owned(&self) -> Result<String, DisabledHotkeysError> {
            Ok(self.letters.lock().unwrap().clone())
        }

        fn write_owned(&self, letters: &str) -> Result<(), DisabledHotkeysError> {
            *self.letters.lock().unwrap() = letters.to_owned();
            Ok(())
        }
    }

    fn letters(values: &str) -> BTreeSet<char> {
        values.chars().collect()
    }

    fn action(
        binding: &str,
        force: bool,
        enabled: bool,
        available: bool,
    ) -> HotkeyRegistrationSnapshot {
        HotkeyRegistrationSnapshot {
            action_id: HotkeyActionId::ClipboardOpenPanel,
            binding: binding.to_owned(),
            configured_enabled: enabled,
            force_override_system: force,
            action_available: available,
            classification: HotkeyBindingClassification::SystemReserved,
            runtime_state: HotkeyRuntimeState::Registered,
            runtime_backend: None,
            detail: None,
        }
    }

    #[test]
    fn win_single_letter_matches_only_bare_win_alphanumerics() {
        assert_eq!(win_single_letter("Win+V"), Some('V'));
        assert_eq!(win_single_letter("Win+1"), Some('1'));
        assert_eq!(win_single_letter("win+v"), Some('V'));
        assert_eq!(win_single_letter("Win+Shift+S"), None);
        assert_eq!(win_single_letter("Alt+Space"), None);
        assert_eq!(win_single_letter("F1"), None);
        assert_eq!(win_single_letter("Win+Enter"), None);
    }

    #[test]
    fn desired_letters_require_enabled_available_forced_system_win_letter() {
        let snapshot = HotkeySnapshot {
            revision: 1,
            actions: vec![
                action("Win+V", true, true, true),
                action("Win+R", true, true, true),
                // disabled -> ignored
                action("Win+E", true, false, true),
                // not forced -> ignored
                action("Win+D", false, true, true),
                // action unavailable -> ignored
                action("Win+X", true, true, false),
                // non Win+letter -> ignored
                action("Win+Shift+S", true, true, true),
            ],
        };

        assert_eq!(desired_disabled_letters(&snapshot), letters("RV"));
    }

    #[test]
    fn reconcile_preserves_user_letters_and_only_appends_new_ones() {
        let (value, owned) = reconcile_letters(&['S', 'Q', 'F'], &BTreeSet::new(), &letters("V"));
        assert_eq!(value, vec!['S', 'Q', 'F', 'V']);
        assert_eq!(owned, letters("V"));
    }

    #[test]
    fn reconcile_removes_only_owned_letters_when_no_longer_desired() {
        let (value, owned) =
            reconcile_letters(&['S', 'Q', 'F', 'V'], &letters("V"), &BTreeSet::new());
        assert_eq!(value, vec!['S', 'Q', 'F']);
        assert!(owned.is_empty());
    }

    #[test]
    fn reconcile_does_not_claim_user_managed_letter_that_matches_desired() {
        // The user already disabled V manually; OpenDeskTools should not claim it.
        let (value, owned) = reconcile_letters(&['V'], &BTreeSet::new(), &letters("V"));
        assert_eq!(value, vec!['V']);
        assert!(owned.is_empty());
    }

    #[test]
    fn manager_adds_letter_and_records_ownership() {
        let registry = Arc::new(FakeRegistry::default());
        let owned = Arc::new(FakeOwned::default());
        let disabler = SystemHotkeyDisabler::new(registry.clone(), owned.clone());

        let outcome = disabler.reconcile(&letters("V")).unwrap();

        assert!(outcome.changed);
        assert_eq!(outcome.registry_value, "V");
        assert_eq!(registry.snapshot().as_deref(), Some("V"));
        assert_eq!(owned.value(), "V");
    }

    #[test]
    fn manager_merges_with_user_letters_and_restores_only_its_own() {
        let registry = Arc::new(FakeRegistry::with_value("SQF"));
        let owned = Arc::new(FakeOwned::default());
        let disabler = SystemHotkeyDisabler::new(registry.clone(), owned.clone());

        disabler.reconcile(&letters("V")).unwrap();
        assert_eq!(registry.snapshot().as_deref(), Some("SQFV"));
        assert_eq!(owned.value(), "V");

        let outcome = disabler.restore_all().unwrap();
        assert!(outcome.changed);
        assert_eq!(registry.snapshot().as_deref(), Some("SQF"));
        assert_eq!(owned.value(), "");
    }

    #[test]
    fn manager_leaves_user_disabled_letter_untouched_on_restore() {
        let registry = Arc::new(FakeRegistry::with_value("V"));
        let owned = Arc::new(FakeOwned::default());
        let disabler = SystemHotkeyDisabler::new(registry.clone(), owned.clone());

        // Desired matches an existing user-managed letter: no change, no ownership.
        let outcome = disabler.reconcile(&letters("V")).unwrap();
        assert!(!outcome.changed);
        assert_eq!(owned.value(), "");

        // Restoring must not delete the letter the user set themselves.
        disabler.restore_all().unwrap();
        assert_eq!(registry.snapshot().as_deref(), Some("V"));
    }

    #[test]
    fn manager_reconcile_is_idempotent() {
        let registry = Arc::new(FakeRegistry::default());
        let owned = Arc::new(FakeOwned::with(""));
        let disabler = SystemHotkeyDisabler::new(registry.clone(), owned.clone());

        disabler.reconcile(&letters("V")).unwrap();
        let outcome = disabler.reconcile(&letters("V")).unwrap();

        assert!(!outcome.changed);
        assert_eq!(registry.write_count(), 1);
        assert_eq!(registry.clear_count(), 0);
    }

    #[test]
    fn manager_clears_registry_value_when_no_letters_remain() {
        let registry = Arc::new(FakeRegistry::with_value("V"));
        let owned = Arc::new(FakeOwned::with("V"));
        let disabler = SystemHotkeyDisabler::new(registry.clone(), owned.clone());

        disabler.restore_all().unwrap();

        assert_eq!(registry.snapshot(), None);
        assert_eq!(registry.clear_count(), 1);
        assert_eq!(owned.value(), "");
    }
}
