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
