//! Log records for the host: a C callback, or nothing until one is set.
use std::ffi::{CString, c_char, c_void};
use std::sync::RwLock;

pub const ERROR: i32 = 1;
pub const WARN: i32 = 2;
pub const INFO: i32 = 3;
pub const DEBUG: i32 = 4;

pub type Callback =
    extern "C" fn(level: i32, target: *const c_char, message: *const c_char, context: *mut c_void);

struct Sink {
    callback: Callback,
    context: usize,
    max_level: i32,
}

static SINK: RwLock<Option<Sink>> = RwLock::new(None);

pub fn set(callback: Option<Callback>, context: *mut c_void, max_level: i32) {
    let sink = callback.map(|callback| Sink {
        callback,
        context: context as usize,
        max_level,
    });
    match SINK.write() {
        Ok(mut s) => *s = sink,
        Err(p) => *p.into_inner() = sink,
    }
}

pub fn emit(level: i32, message: &str) {
    let Ok(sink) = SINK.read() else { return };
    let Some(sink) = sink.as_ref() else { return };
    if level > sink.max_level {
        return;
    }
    let message = CString::new(message.replace('\0', " ")).unwrap_or_default();
    (sink.callback)(
        level,
        c"cmvs".as_ptr(),
        message.as_ptr(),
        sink.context as *mut c_void,
    );
}
