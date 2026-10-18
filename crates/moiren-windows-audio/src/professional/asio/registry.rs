//! Native registry enumeration, following Steinberg's host/pc/asiolist.cpp.
//! Raw Win32 declarations avoid adding a windows crate Registry feature solely
//! for these four stable APIs. No registry command or external SDK is executed.

use std::{collections::BTreeMap, ffi::c_void, ptr};
use windows::core::GUID;

use super::{AsioDriver, BackendError};

type HKey = *mut c_void;
const KEY_READ_64: u32 = 0x20019 | 0x0100;
const NOT_FOUND: i32 = 2;
const NO_MORE_ITEMS: i32 = 259;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegOpenKeyExW(
        key: HKey,
        subkey: *const u16,
        options: u32,
        access: u32,
        result: *mut HKey,
    ) -> i32;
    fn RegCloseKey(key: HKey) -> i32;
    fn RegEnumKeyExW(
        key: HKey,
        index: u32,
        name: *mut u16,
        name_length: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_length: *mut u32,
        last_write: *mut c_void,
    ) -> i32;
    fn RegQueryValueExW(
        key: HKey,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        bytes: *mut u32,
    ) -> i32;
}

struct Key(HKey);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: only successful RegOpenKeyExW results are wrapped.
        unsafe { RegCloseKey(self.0) };
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn open(parent: HKey, name: &str) -> Result<Option<Key>, BackendError> {
    let mut key = ptr::null_mut();
    // SAFETY: the UTF-16 name is terminated and result points to writable memory.
    let code = unsafe { RegOpenKeyExW(parent, wide(name).as_ptr(), 0, KEY_READ_64, &mut key) };
    match code {
        0 => Ok(Some(Key(key))),
        NOT_FOUND => Ok(None),
        _ => Err(BackendError::Failed(format!(
            "ASIO registry open {name:?} failed (Win32 {code})"
        ))),
    }
}

fn string(key: &Key, name: &str) -> Option<String> {
    let name = wide(name);
    let mut bytes = 0;
    let mut kind = 0;
    // SAFETY: first call only queries size/type. The second call's writable
    // buffer is capped and allocated off the audio thread.
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null_mut(),
            &mut kind,
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if code != 0 || !matches!(kind, 1 | 2) || !(2..=65536).contains(&bytes) || bytes % 2 != 0 {
        return None;
    }
    let mut value = vec![0u16; bytes as usize / 2];
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null_mut(),
            &mut kind,
            value.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if code != 0 || !matches!(kind, 1 | 2) || bytes % 2 != 0 || bytes as usize > value.len() * 2 {
        return None;
    }
    value.truncate(bytes as usize / 2);
    let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    String::from_utf16(&value[..end]).ok()
}

fn has_inproc_server(clsid: &str) -> Result<bool, BackendError> {
    let Some(key) = open(
        0x8000_0000u32 as i32 as isize as HKey,
        &format!("CLSID\\{clsid}\\InprocServer32"),
    )?
    else {
        return Ok(false);
    };
    Ok(string(&key, "").is_some_and(|path| !path.trim().is_empty()))
}

pub(super) fn parse_id(id: &str) -> Result<GUID, BackendError> {
    let text = id
        .strip_prefix('{')
        .and_then(|text| text.strip_suffix('}'))
        .unwrap_or(id);
    GUID::try_from(text).map_err(|_| {
        BackendError::Unsupported(format!("ASIO driver ID {id:?} is not a valid CLSID"))
    })
}

pub(super) fn canonical_id(id: &GUID) -> String {
    format!("{{{id:?}}}")
}

pub(super) fn list() -> Result<Vec<AsioDriver>, BackendError> {
    let mut drivers = BTreeMap::new();
    // Win32 predefined HKEY constants are sign-extended LONG_PTR values.
    // Per-user registrations override machine descriptions for the same CLSID.
    for root in [0x8000_0002u32, 0x8000_0001u32] {
        let Some(key) = open(root as i32 as isize as HKey, "SOFTWARE\\ASIO")? else {
            continue;
        };
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut length = name.len() as u32;
            // SAFETY: registry key names are at most 255 UTF-16 code units.
            let code = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            if code == NO_MORE_ITEMS {
                break;
            }
            if code != 0 {
                return Err(BackendError::Failed(format!(
                    "ASIO registry enumeration failed (Win32 {code})"
                )));
            }
            let name = String::from_utf16_lossy(&name[..length as usize]);
            let Some(driver) = open(key.0, &name)? else {
                continue; // The registration can disappear during discovery.
            };
            let Some(clsid) = string(&driver, "CLSID").and_then(|s| parse_id(&s).ok()) else {
                continue; // Ignore broken/non-driver registrations.
            };
            let id = canonical_id(&clsid);
            if !has_inproc_server(&id)? {
                continue; // Match Steinberg's host enumeration: CLSID + COM server.
            }
            let name = string(&driver, "Description")
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(name);
            drivers.insert(id.clone(), AsioDriver { id, name });
        }
    }
    Ok(drivers.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_ids_keep_the_clsid_identity_across_case_and_registry_order() {
        let id = parse_id("{12345678-9abc-def0-1234-56789abcdef0}").unwrap();
        assert_eq!(canonical_id(&id), "{12345678-9ABC-DEF0-1234-56789ABCDEF0}");
        assert_eq!(
            parse_id("12345678-9ABC-DEF0-1234-56789ABCDEF0").unwrap(),
            id
        );
        assert!(parse_id("0").is_err());
        assert!(parse_id("{not-a-driver}").is_err());
    }
}
