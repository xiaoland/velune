//! Private synchronous C boundary; no Go pointers or callbacks cross it.
#![allow(unsafe_code)]
use super::{TranslationError, TranslationErrorKind, invalid};
use serde_json::Value;
use std::{
    cell::Cell,
    ffi::{CStr, c_char},
    marker::PhantomData,
};
unsafe extern "C" {
    fn velune_relaykit_call(handle: usize, data: *const c_char, size: usize) -> *mut c_char;
    fn velune_relaykit_release(handle: usize);
    fn velune_relaykit_free(data: *mut c_char);
}

pub(super) fn call(handle: usize, command: Value) -> Result<Value, TranslationError> {
    let bytes = command.to_string();
    // The synchronous bridge copies input and returns a C allocation, never a Go pointer.
    let output = unsafe { velune_relaykit_call(handle, bytes.as_ptr().cast(), bytes.len()) };
    if output.is_null() {
        return Err(invalid("relaykit.allocation"));
    }
    let result = unsafe { serde_json::from_slice::<Value>(CStr::from_ptr(output).to_bytes()) };
    unsafe {
        velune_relaykit_free(output);
    }
    let result = result.map_err(|_| invalid("relaykit.output"))?;
    if let Some(error) = result["error"].as_str() {
        return Err(if error == "truncated" {
            TranslationError {
                kind: TranslationErrorKind::Truncated,
                field: "stream.missing_terminal",
            }
        } else {
            invalid("relaykit.conversion")
        });
    }
    Ok(result)
}

pub(super) struct Session {
    pub(super) handle: usize,
    _not_sync: PhantomData<Cell<()>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        // One owned handle moves from PreparedTranslation to StreamTranslator.
        unsafe {
            velune_relaykit_release(self.handle);
        }
    }
}

impl Session {
    pub(super) fn new(handle: usize) -> Self {
        Self {
            handle,
            _not_sync: PhantomData,
        }
    }
}
