//! The C interface for app hosts: `platform/ios/include/cmvs_mobile.h`.
//!
//! A session is a [`session::Session`]: the engine's `Startup` with fonts, input,
//! a pulled audio mixer and wgpu presentation into the host's view. The host steps
//! it once per display frame. Every entry point catches panics and reports them as
//! errors; all but the `cmvs_audio_*` calls belong on the thread that opened the
//! session.
#![allow(clippy::missing_safety_doc)]

pub mod audio;
mod backend;
pub mod input;
pub mod log;
pub mod patch;
pub mod select;
pub mod session;

// The desktop host's font code, compiled here unchanged so upstream fixes arrive
// with every sync: face lookup by name and glyph rasterising with ab_glyph.
#[path = "../../cmvs-desktop/src/font.rs"]
#[allow(clippy::all)]
pub mod font;
#[path = "../../cmvs-desktop/src/font_library.rs"]
#[allow(clippy::all)]
pub mod font_library;

use session::{Config, Encoding, FontFile, Session, Status};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::Mutex,
};

/// `CMVS_ABI_VERSION`.
pub const ABI_VERSION: u32 = 2;

/// `CmvsConfig`.
#[repr(C)]
pub struct CmvsConfig {
    pub struct_size: u32,
    pub game_dir: *const c_char,
    pub save_dir: *const c_char,
    pub encoding: u32,
    pub utc_offset_ms: i64,
}

/// `CmvsSession`.
pub struct CmvsSession(Session);
/// `CmvsAudio`.
pub struct CmvsAudio(audio::AudioOutput);

const RUNNING: i32 = 0;
const ENDED: i32 = 1;
const FAILED: i32 = 2;

const POINTER_MOVE: i32 = 0;
const POINTER_LEFT_DOWN: i32 = 1;
const POINTER_LEFT_UP: i32 = 2;
const POINTER_RIGHT_DOWN: i32 = 3;
const POINTER_RIGHT_UP: i32 = 4;
const POINTER_MIDDLE_DOWN: i32 = 5;
const POINTER_MIDDLE_UP: i32 = 6;

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}
static FONTS: Mutex<Vec<FontFile>> = Mutex::new(Vec::new());

fn set_error(message: impl Into<String>) {
    let message = message.into().replace('\0', " ");
    log::emit(log::ERROR, &message);
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::new(message).unwrap_or_default());
}
fn clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::default());
}

/// Runs `f`, turning a panic into an error and `fallback`.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(p) => {
            let message = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown".into());
            set_error(format!("panic: {message}"));
            fallback
        }
    }
}

/// A borrowed C string; None for NULL or empty.
///
/// # Safety
/// `p` is NULL or a NUL-terminated string.
#[allow(unsafe_code)]
unsafe fn string(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller passes a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    (!s.is_empty()).then_some(s)
}

fn status_code(status: &Status) -> i32 {
    match status {
        Status::Running => RUNNING,
        Status::Ended => ENDED,
        Status::Failed(message) => {
            LAST_ERROR.with(|e| {
                *e.borrow_mut() = CString::new(message.replace('\0', " ")).unwrap_or_default()
            });
            FAILED
        }
    }
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn cmvs_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn cmvs_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn cmvs_set_log_callback(
    callback: Option<log::Callback>,
    context: *mut c_void,
    max_level: i32,
) {
    guard((), || log::set(callback, context, max_level))
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_register_font(path: *const c_char, face: *const c_char) -> i32 {
    guard(0, || {
        clear_error();
        // SAFETY: C strings from the host.
        let (Some(path), face) = (unsafe { string(path) }, unsafe { string(face) }) else {
            set_error("no font path");
            return 0;
        };
        let path = PathBuf::from(path);
        if !path.is_file() {
            set_error(format!("font {} is missing", path.display()));
            return 0;
        }
        match FONTS.lock() {
            Ok(mut fonts) => fonts.push(FontFile { path, face }),
            Err(p) => p.into_inner().push(FontFile { path, face }),
        }
        1
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn cmvs_clear_fonts() {
    guard((), || match FONTS.lock() {
        Ok(mut fonts) => fonts.clear(),
        Err(p) => p.into_inner().clear(),
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_open(config: *const CmvsConfig) -> *mut CmvsSession {
    guard(std::ptr::null_mut(), || {
        clear_error();
        // SAFETY: the host passes a live config of at least `struct_size` bytes.
        let Some(config) = (unsafe { config.as_ref() }) else {
            set_error("no config");
            return std::ptr::null_mut();
        };
        if (config.struct_size as usize) < std::mem::size_of::<CmvsConfig>() {
            set_error("config is from an older header");
            return std::ptr::null_mut();
        }
        // SAFETY: C strings from the host.
        let Some(game_dir) = (unsafe { string(config.game_dir) }) else {
            set_error("no game folder");
            return std::ptr::null_mut();
        };
        let encoding = match config.encoding {
            0 => Encoding::Automatic,
            1 => Encoding::ShiftJis,
            2 => Encoding::Gbk,
            other => {
                set_error(format!("unknown encoding {other}"));
                return std::ptr::null_mut();
            }
        };
        let fonts = match FONTS.lock() {
            Ok(f) => f.clone(),
            Err(p) => p.into_inner().clone(),
        };
        let config = Config {
            game_dir: game_dir.into(),
            // SAFETY: as above.
            save_dir: unsafe { string(config.save_dir) }.map(PathBuf::from),
            encoding,
            utc_offset_ms: config.utc_offset_ms,
            fonts,
        };
        match Session::open(config) {
            Ok(session) => Box::into_raw(Box::new(CmvsSession(session))),
            Err(e) => {
                set_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_screen_size(
    _session: *const CmvsSession,
    width: *mut u32,
    height: *mut u32,
) {
    // SAFETY: NULL or writable pointers from the host.
    unsafe {
        if let Some(w) = width.as_mut() {
            *w = input::SCREEN[0];
        }
        if let Some(h) = height.as_mut() {
            *h = input::SCREEN[1];
        }
    }
}

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
fn surface_target(view: *mut c_void) -> Option<cmvs_wgpu::wgpu::SurfaceTargetUnsafe> {
    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, UiKitDisplayHandle, UiKitWindowHandle,
    };
    let view = std::ptr::NonNull::new(view)?;
    Some(cmvs_wgpu::wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::UiKit(UiKitDisplayHandle::new())),
        raw_window_handle: RawWindowHandle::UiKit(UiKitWindowHandle::new(view)),
    })
}

#[cfg(target_os = "macos")]
fn surface_target(view: *mut c_void) -> Option<cmvs_wgpu::wgpu::SurfaceTargetUnsafe> {
    use raw_window_handle::{
        AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    let view = std::ptr::NonNull::new(view)?;
    Some(cmvs_wgpu::wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
        raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(view)),
    })
}

#[cfg(not(target_vendor = "apple"))]
fn surface_target(_view: *mut c_void) -> Option<cmvs_wgpu::wgpu::SurfaceTargetUnsafe> {
    None
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_set_surface(
    session: *mut CmvsSession,
    ui_view: *mut c_void,
    width_px: u32,
    height_px: u32,
) -> i32 {
    guard(-1, || {
        clear_error();
        // SAFETY: a session from cmvs_open, used on its thread.
        let Some(CmvsSession(s)) = (unsafe { session.as_mut() }) else {
            set_error("no session");
            return -1;
        };
        let result = s.attach(ui_view as usize, [width_px, height_px], |instance| {
            let target =
                surface_target(ui_view).ok_or("views are not supported on this platform")?;
            // SAFETY: the host keeps the view alive until it detaches or closes.
            Ok(unsafe { instance.create_surface_unsafe(target) }?)
        });
        match result {
            Ok(()) => 0,
            Err(e) => {
                set_error(e.to_string());
                -1
            }
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_detach_surface(session: *mut CmvsSession) {
    guard((), || {
        // SAFETY: as in cmvs_set_surface.
        if let Some(CmvsSession(s)) = unsafe { session.as_mut() } {
            s.detach();
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_step(session: *mut CmvsSession, dt_ms: u32) -> i32 {
    guard(FAILED, || {
        // SAFETY: as in cmvs_set_surface.
        let Some(CmvsSession(s)) = (unsafe { session.as_mut() }) else {
            set_error("no session");
            return FAILED;
        };
        status_code(&s.step(dt_ms))
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_pointer(session: *mut CmvsSession, phase: i32, x: f32, y: f32) {
    guard((), || {
        // SAFETY: as in cmvs_set_surface.
        let Some(CmvsSession(s)) = (unsafe { session.as_mut() }) else {
            return;
        };
        s.pointer(f64::from(x), f64::from(y));
        let (code, down) = match phase {
            POINTER_LEFT_DOWN => (input::BUTTON_LEFT, true),
            POINTER_LEFT_UP => (input::BUTTON_LEFT, false),
            POINTER_RIGHT_DOWN => (input::BUTTON_RIGHT, true),
            POINTER_RIGHT_UP => (input::BUTTON_RIGHT, false),
            POINTER_MIDDLE_DOWN => (input::BUTTON_MIDDLE, true),
            POINTER_MIDDLE_UP => (input::BUTTON_MIDDLE, false),
            POINTER_MOVE => return,
            _ => return,
        };
        s.button(code, down);
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_wheel(session: *mut CmvsSession, notches: i32) {
    guard((), || {
        // SAFETY: as in cmvs_set_surface.
        if let Some(CmvsSession(s)) = unsafe { session.as_mut() } {
            s.wheel(notches);
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_key(session: *mut CmvsSession, vk: i32, down: i32) {
    guard((), || {
        // SAFETY: as in cmvs_set_surface.
        if let Some(CmvsSession(s)) = unsafe { session.as_mut() } {
            s.key(vk as u32, down != 0);
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_set_focused(session: *mut CmvsSession, focused: i32) {
    guard((), || {
        // SAFETY: as in cmvs_set_surface.
        if let Some(CmvsSession(s)) = unsafe { session.as_mut() } {
            s.set_focused(focused != 0);
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_audio(session: *mut CmvsSession) -> *mut CmvsAudio {
    guard(std::ptr::null_mut(), || {
        clear_error();
        // SAFETY: as in cmvs_set_surface.
        let Some(CmvsSession(s)) = (unsafe { session.as_mut() }) else {
            set_error("no session");
            return std::ptr::null_mut();
        };
        let Some(source) = s.take_audio_source() else {
            set_error("the session's audio output was already taken");
            return std::ptr::null_mut();
        };
        match audio::AudioOutput::new(source) {
            Ok(out) => Box::into_raw(Box::new(CmvsAudio(out))),
            Err(e) => {
                set_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn cmvs_audio_sample_rate() -> u32 {
    audio::SAMPLE_RATE
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_audio_render(audio: *mut CmvsAudio, out: *mut f32, frames: u32) {
    // No guard: render never panics, and catch_unwind is not free on an audio thread.
    if out.is_null() {
        return;
    }
    // SAFETY: the host passes a buffer of 2 * frames floats.
    let out = unsafe { std::slice::from_raw_parts_mut(out, 2 * frames as usize) };
    // SAFETY: an output from cmvs_audio, rendered from one thread at a time.
    match unsafe { audio.as_mut() } {
        Some(CmvsAudio(a)) => a.render(out),
        None => out.fill(0.0),
    }
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_audio_release(audio: *mut CmvsAudio) {
    if !audio.is_null() {
        // SAFETY: an output from cmvs_audio, released once.
        drop(unsafe { Box::from_raw(audio) });
    }
}

#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "C" fn cmvs_close(session: *mut CmvsSession) {
    guard((), || {
        if !session.is_null() {
            // SAFETY: a session from cmvs_open, closed once.
            let CmvsSession(s) = *unsafe { Box::from_raw(session) };
            s.close();
        }
    })
}
