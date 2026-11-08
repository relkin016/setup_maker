//! Registry helpers.
#![allow(dead_code)]

use crate::w::wide;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Registry::*;

pub fn root_of(name: &str, admin: bool) -> Option<HKEY> {
    Some(match name.trim().to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        "HKU" | "HKEY_USERS" => HKEY_USERS,
        "HKCC" | "HKEY_CURRENT_CONFIG" => HKEY_CURRENT_CONFIG,
        "HKA" => {
            if admin {
                HKEY_LOCAL_MACHINE
            } else {
                HKEY_CURRENT_USER
            }
        }
        _ => return None,
    })
}

pub fn root_name(name: &str, admin: bool) -> String {
    let n = name.trim().to_ascii_uppercase();
    if n == "HKA" {
        if admin { "HKLM" } else { "HKCU" }.to_string()
    } else {
        n
    }
}

pub struct Key(pub HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

pub fn open(root: HKEY, sub: &str, write: bool) -> Option<Key> {
    let s = wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    let access = if write { KEY_ALL_ACCESS } else { KEY_READ };
    let r = unsafe { RegOpenKeyExW(root, s.as_ptr(), 0, access, &mut h) };
    if r == ERROR_SUCCESS {
        Some(Key(h))
    } else {
        None
    }
}

/// Create (or open) a key. Returns (key, created_new).
pub fn create(root: HKEY, sub: &str) -> Option<(Key, bool)> {
    let s = wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    let mut disp = 0u32;
    let r = unsafe {
        RegCreateKeyExW(root, s.as_ptr(), 0, std::ptr::null(), 0, KEY_ALL_ACCESS, std::ptr::null(), &mut h, &mut disp)
    };
    if r == ERROR_SUCCESS {
        Some((Key(h), disp == REG_CREATED_NEW_KEY))
    } else {
        None
    }
}

pub fn key_exists(root: HKEY, sub: &str) -> bool {
    open(root, sub, false).is_some()
}

pub fn value_exists(k: &Key, name: &str) -> bool {
    let n = wide(name);
    unsafe { RegQueryValueExW(k.0, n.as_ptr(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) == ERROR_SUCCESS }
}

pub fn set_raw(k: &Key, name: &str, ty: REG_VALUE_TYPE, data: &[u8]) -> bool {
    let n = wide(name);
    unsafe { RegSetValueExW(k.0, n.as_ptr(), 0, ty, data.as_ptr(), data.len() as u32) == ERROR_SUCCESS }
}

pub fn set_string(k: &Key, name: &str, v: &str) -> bool {
    let w = wide(v);
    let bytes = unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2) };
    set_raw(k, name, REG_SZ, bytes)
}

pub fn set_dword(k: &Key, name: &str, v: u32) -> bool {
    set_raw(k, name, REG_DWORD, &v.to_le_bytes())
}

/// Write a value of an Inno-style type with already expanded data.
pub fn set_typed(k: &Key, name: &str, ty: &str, data: &str) -> bool {
    match ty.to_ascii_lowercase().as_str() {
        "string" | "sz" => set_string(k, name, data),
        "expandsz" => {
            let w = wide(data);
            let b = unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2) };
            set_raw(k, name, REG_EXPAND_SZ, b)
        }
        "multisz" => {
            // "{break}" separates entries, like Inno
            let mut w: Vec<u16> = Vec::new();
            for part in data.split("{break}") {
                w.extend(part.encode_utf16());
                w.push(0);
            }
            w.push(0);
            let b = unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2) };
            set_raw(k, name, REG_MULTI_SZ, b)
        }
        "dword" => set_dword(k, name, parse_num(data) as u32),
        "qword" => set_raw(k, name, REG_QWORD, &parse_num(data).to_le_bytes()),
        "binary" => {
            let b: Vec<u8> = data
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .filter_map(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .collect();
            set_raw(k, name, REG_BINARY, &b)
        }
        _ => true, // "none": key only
    }
}

fn parse_num(s: &str) -> u64 {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("$").or_else(|| s.strip_prefix("0x")) {
        u64::from_str_radix(h, 16).unwrap_or(0)
    } else {
        s.parse::<i64>().map(|v| v as u64).unwrap_or(0)
    }
}

pub fn get_string(k: &Key, name: &str) -> Option<String> {
    let n = wide(name);
    let mut ty = 0u32;
    let mut len = 0u32;
    unsafe {
        if RegQueryValueExW(k.0, n.as_ptr(), std::ptr::null(), &mut ty, std::ptr::null_mut(), &mut len) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (len as usize / 2) + 2];
        let mut l2 = (buf.len() * 2) as u32;
        if RegQueryValueExW(k.0, n.as_ptr(), std::ptr::null(), &mut ty, buf.as_mut_ptr() as *mut u8, &mut l2) != ERROR_SUCCESS {
            return None;
        }
        if ty == REG_DWORD {
            return Some(u32::from_le_bytes([buf[0] as u8, (buf[0] >> 8) as u8, buf[1] as u8, (buf[1] >> 8) as u8]).to_string());
        }
        Some(crate::w::from_wide_buf(&buf))
    }
}

pub fn delete_value(k: &Key, name: &str) -> bool {
    let n = wide(name);
    unsafe { RegDeleteValueW(k.0, n.as_ptr()) == ERROR_SUCCESS }
}

pub fn delete_tree(root: HKEY, sub: &str) -> bool {
    let s = wide(sub);
    unsafe { RegDeleteTreeW(root, s.as_ptr()) == ERROR_SUCCESS || RegDeleteKeyW(root, s.as_ptr()) == ERROR_SUCCESS }
}

pub fn delete_if_empty(root: HKEY, sub: &str) -> bool {
    let Some(k) = open(root, sub, false) else { return false };
    let (mut subkeys, mut values) = (0u32, 0u32);
    unsafe {
        RegQueryInfoKeyW(
            k.0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut subkeys,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut values,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
    drop(k);
    if subkeys == 0 && values == 0 {
        let s = wide(sub);
        unsafe { RegDeleteKeyW(root, s.as_ptr()) == ERROR_SUCCESS }
    } else {
        false
    }
}

/// `HKLM\Software\X` + value -> string (for `{reg:...}` and RegKeyExists).
pub fn read_string(full_key: &str, value: &str) -> Option<String> {
    let (root, sub) = full_key.split_once('\\')?;
    let root = root_of(root, crate::w::is_admin())?;
    let k = open(root, sub, false)?;
    get_string(&k, value)
}

pub fn full_key_exists(full_key: &str) -> bool {
    match full_key.split_once('\\') {
        Some((r, s)) => root_of(r, crate::w::is_admin()).map(|r| key_exists(r, s)).unwrap_or(false),
        None => false,
    }
}
