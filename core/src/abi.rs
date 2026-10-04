//! Stable C ABI for embedding the cross-platform CoreRuntime.
//!
//! All payloads are UTF-8 JSON strings owned by Rust until the caller calls
//! velune_core_string_free. Calls on one handle are synchronous and must be
//! serialized by the platform client.

use crate::runtime::{CoreRuntime, RuntimeError, runtime_options};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::Mutex,
};

pub struct VeluneCore {
    runtime: Mutex<CoreRuntime>,
}

fn string_ptr(value: String) -> *mut c_char {
    let sanitized = value.replace('\0', "\\u0000");
    CString::new(sanitized).expect("NUL was removed").into_raw()
}

fn envelope(error: impl Into<String>) -> *mut c_char {
    string_ptr(
        json!({
            "ok": false,
            "error": {"code":"abi", "message": error.into()}
        })
        .to_string(),
    )
}

unsafe fn input_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("null JSON pointer".into());
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| "JSON is not UTF-8".into())
}

#[unsafe(no_mangle)]
pub extern "C" fn velune_core_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
/// # Safety
/// `options_json` must point to a valid NUL-terminated UTF-8 string for the
/// duration of the call, and `out_handle` must be writable when non-null.
pub unsafe extern "C" fn velune_core_open(
    options_json: *const c_char,
    out_handle: *mut *mut VeluneCore,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_handle.is_null() {
            return envelope("null output handle");
        }
        // SAFETY: out_handle was validated non-null and is owned by the caller.
        unsafe { *out_handle = ptr::null_mut() };
        // SAFETY: the caller owns a NUL-terminated options string for this call.
        let input = match unsafe { input_string(options_json) } {
            Ok(input) => input,
            Err(error) => return envelope(error),
        };
        let options: Value = match serde_json::from_str(&input) {
            Ok(value) => value,
            Err(error) => return envelope(format!("options JSON: {error}")),
        };
        let options = match runtime_options(&options) {
            Ok(options) => options,
            Err(error) => return runtime_error(error),
        };
        let runtime = match CoreRuntime::open(options) {
            Ok(runtime) => runtime,
            Err(error) => return runtime_error(error),
        };
        let response = json!({
            "ok": true,
            "data": {"abiVersion": 1, "contractVersion": 3}
        });
        let handle = Box::new(VeluneCore {
            runtime: Mutex::new(runtime),
        });
        // SAFETY: out_handle was validated and the Box remains owned by the caller.
        unsafe { *out_handle = Box::into_raw(handle) };
        string_ptr(response.to_string())
    }));
    result.unwrap_or_else(|_| envelope("panic while opening CoreRuntime"))
}

#[unsafe(no_mangle)]
/// # Safety
/// `handle` must be a live handle returned by `velune_core_open`, and
/// `request_json` must point to a valid NUL-terminated UTF-8 string for the
/// duration of this synchronous call.
pub unsafe extern "C" fn velune_core_request(
    handle: *mut VeluneCore,
    request_json: *const c_char,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return envelope("null CoreRuntime handle");
        }
        // SAFETY: handle is borrowed for this synchronous call and not freed
        // concurrently by the platform queue.
        let runtime = unsafe { &*handle };
        // SAFETY: the caller owns a NUL-terminated request string for this call.
        let input = match unsafe { input_string(request_json) } {
            Ok(input) => input,
            Err(error) => return envelope(error),
        };
        let request: Value = match serde_json::from_str(&input) {
            Ok(value) => value,
            Err(error) => return envelope(format!("request JSON: {error}")),
        };
        let mut runtime = match runtime.runtime.lock() {
            Ok(runtime) => runtime,
            Err(_) => return envelope("CoreRuntime lock poisoned"),
        };
        string_ptr(runtime.request(&request).to_string())
    }));
    result.unwrap_or_else(|_| envelope("panic while requesting CoreRuntime"))
}

#[unsafe(no_mangle)]
/// # Safety
/// `handle` must point to a caller-owned handle pointer. A successful close
/// consumes that handle and writes null through the pointer.
pub unsafe extern "C" fn velune_core_close(handle: *mut *mut VeluneCore) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return envelope("null handle pointer");
        }
        // SAFETY: handle is a caller-owned pointer to an opaque handle.
        let current = unsafe { *handle };
        if current.is_null() {
            return string_ptr(json!({"ok":true,"data":{"closed":true}}).to_string());
        }
        // SAFETY: current remains owned by caller unless the close succeeds.
        let runtime = unsafe { &*current };
        let mut runtime = match runtime.runtime.lock() {
            Ok(runtime) => runtime,
            Err(_) => return envelope("CoreRuntime lock poisoned"),
        };
        if let Err(error) = runtime.close_if_idle() {
            return runtime_error(error);
        }
        drop(runtime);
        // SAFETY: close succeeds only after the runtime is idle and current is
        // exclusively consumed by this call.
        unsafe {
            drop(Box::from_raw(current));
            *handle = ptr::null_mut();
        }
        string_ptr(json!({"ok":true,"data":{"closed":true}}).to_string())
    }));
    result.unwrap_or_else(|_| envelope("panic while closing CoreRuntime"))
}

#[unsafe(no_mangle)]
/// # Safety
/// `value` must be null or a string pointer returned by this ABI and must be
/// passed exactly once.
pub unsafe extern "C" fn velune_core_string_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: value must be a pointer returned by this module exactly once.
        unsafe { drop(CString::from_raw(value)) };
    }
}

fn runtime_error(error: RuntimeError) -> *mut c_char {
    string_ptr(error.envelope().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::CString, fs};
    use tempfile::tempdir;

    fn options(dir: &std::path::Path) -> CString {
        CString::new(
            json!({
                "homeDirectory":dir.join("home"),
                "resourcesDirectory":dir.join("resources"),
                "credentialResolver":null
            })
            .to_string(),
        )
        .unwrap()
    }

    unsafe fn take(value: *mut c_char) -> Value {
        assert!(!value.is_null());
        // SAFETY: value was returned by the ABI and is consumed exactly once.
        let value = unsafe { CString::from_raw(value) };
        serde_json::from_str(value.to_str().unwrap()).unwrap()
    }

    #[test]
    fn ffi_open_request_close_owns_json_and_handle() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("resources")).unwrap();
        let input = options(dir.path());
        let mut handle = ptr::null_mut();
        let opened = unsafe { velune_core_open(input.as_ptr(), &mut handle) };
        let opened = unsafe { take(opened) };
        assert_eq!(opened["ok"], true);
        assert!(!handle.is_null());
        let request = CString::new(r#"{"version":3,"action":"list"}"#).unwrap();
        let response = unsafe { take(velune_core_request(handle, request.as_ptr())) };
        assert_eq!(response["ok"], true);
        let closed = unsafe { take(velune_core_close(&mut handle)) };
        assert_eq!(closed["ok"], true);
        assert!(handle.is_null());
    }

    #[test]
    fn ffi_rejects_duplicate_home_until_first_handle_closes() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("resources")).unwrap();
        let input = options(dir.path());
        let mut first = ptr::null_mut();
        let opened = unsafe { take(velune_core_open(input.as_ptr(), &mut first)) };
        assert_eq!(opened["ok"], true);
        let mut second = ptr::null_mut();
        let rejected = unsafe { take(velune_core_open(input.as_ptr(), &mut second)) };
        assert_eq!(rejected["ok"], false);
        assert!(second.is_null());
        let _ = unsafe { take(velune_core_close(&mut first)) };
        let mut reopened = ptr::null_mut();
        let opened = unsafe { take(velune_core_open(input.as_ptr(), &mut reopened)) };
        assert_eq!(opened["ok"], true);
        let _ = unsafe { take(velune_core_close(&mut reopened)) };
    }

    #[test]
    fn ffi_open_rejects_relative_paths_without_a_handle() {
        let input =
            CString::new(r#"{"homeDirectory":"relative","resourcesDirectory":"/tmp"}"#).unwrap();
        let mut handle = ptr::null_mut();
        let response = unsafe { take(velune_core_open(input.as_ptr(), &mut handle)) };
        assert_eq!(response["ok"], false);
        assert!(handle.is_null());
    }

    #[test]
    fn ffi_configuration_persists_across_reopen() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("resources")).unwrap();
        let input = options(dir.path());
        let mut handle = ptr::null_mut();
        let _ = unsafe { take(velune_core_open(input.as_ptr(), &mut handle)) };
        let gateway = json!({
            "id":"gateway",
            "name":"Fixture",
            "models":[],
            "providers":[],
            "routes":[],
            "failover":{"mode":"disabled"}
        });
        let request = CString::new(
            json!({
                "version":3,
                "action":"gateways",
                "payload":{"operation":"upsert","gateway":gateway.to_string()}
            })
            .to_string(),
        )
        .unwrap();
        let response = unsafe { take(velune_core_request(handle, request.as_ptr())) };
        assert_eq!(response["ok"], true);
        assert_eq!(unsafe { take(velune_core_close(&mut handle)) }["ok"], true);

        let mut reopened = ptr::null_mut();
        let opened = unsafe { take(velune_core_open(input.as_ptr(), &mut reopened)) };
        assert_eq!(opened["ok"], true, "{opened}");
        let list = CString::new(r#"{"version":3,"action":"list"}"#).unwrap();
        let response = unsafe { take(velune_core_request(reopened, list.as_ptr())) };
        assert_eq!(response["data"]["gateways"][0]["id"], "gateway");
        let _ = unsafe { take(velune_core_close(&mut reopened)) };
    }

    #[test]
    fn ffi_open_rejects_malformed_configuration_without_overwriting_it() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("resources")).unwrap();
        let home = dir.path().join("home");
        fs::create_dir(&home).unwrap();
        let path = home.join("generic-config.json");
        let original = b"{ malformed";
        fs::write(&path, original).unwrap();
        let input = options(dir.path());
        let mut handle = ptr::null_mut();
        let response = unsafe { take(velune_core_open(input.as_ptr(), &mut handle)) };
        assert_eq!(response["ok"], false);
        assert!(handle.is_null());
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
