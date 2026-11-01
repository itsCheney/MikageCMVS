//! Headless check of the mobile session on a desktop: opens a game the way the app
//! does, steps it on a virtual clock, taps the screen now and then, and writes the
//! last frame (CPU-drawn) as a PNG. No window, GPU or audio device.
use cmvs_mobile::session::{Config, Encoding, FontFile, Session, Status};
use std::ffi::{CStr, c_char, c_void};

extern "C" fn print_log(
    level: i32,
    _target: *const c_char,
    message: *const c_char,
    _: *mut c_void,
) {
    // SAFETY: the session passes a NUL-terminated message valid for the call.
    #[allow(unsafe_code)]
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let label = ["?", "error", "warn", "info", "debug"]
        .get(level as usize)
        .unwrap_or(&"?");
    eprintln!("[{label}] {message}");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: cmvs-mobile-run <game-dir> --font <file> [--face <name>] [--save-dir <dir>] \
                 [--seconds <n>] [--tap-every <ms>] [--tap-at <x>,<y>] [--click <ms>:<x>,<y>]... \
                 [--shot <out.png>]";
    let game_dir = args.next().ok_or(usage)?;
    let (mut font, mut face, mut save_dir, mut shot) = (None, None, None, None);
    let (mut seconds, mut tap_every) = (20u32, 0u32);
    let mut tap_at = (640.0f64, 600.0f64);
    let mut clicks: Vec<(u32, (f64, f64))> = Vec::new();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(usage)?;
        match flag.as_str() {
            "--font" => font = Some(value),
            "--face" => face = Some(value),
            "--save-dir" => save_dir = Some(value),
            "--seconds" => seconds = value.parse()?,
            "--tap-every" => tap_every = value.parse()?,
            "--tap-at" => tap_at = point(&value).ok_or(usage)?,
            "--click" => {
                let (ms, at) = value.split_once(':').ok_or(usage)?;
                clicks.push((ms.parse()?, point(at).ok_or(usage)?));
            }
            "--shot" => shot = Some(value),
            _ => return Err(usage.into()),
        }
    }
    cmvs_mobile::log::set(Some(print_log), std::ptr::null_mut(), 3);
    let mut session = Session::open(Config {
        game_dir: game_dir.into(),
        save_dir: save_dir.map(Into::into),
        encoding: Encoding::ShiftJis,
        utc_offset_ms: 0,
        fonts: vec![FontFile {
            path: font.ok_or(usage)?.into(),
            face,
        }],
    })?;
    let frame_ms = 16;
    let mut status = Status::Running;
    for frame in 0..seconds * 1000 / frame_ms {
        let now = frame * frame_ms;
        let mut tap = None;
        if tap_every != 0 && now % tap_every < frame_ms {
            tap = Some(tap_at);
        }
        if let Some(&(_, at)) = clicks
            .iter()
            .find(|(ms, _)| (now..now + frame_ms).contains(ms))
        {
            tap = Some(at);
        }
        if let Some((x, y)) = tap {
            session.pointer(x, y);
            session.button(cmvs_mobile::input::BUTTON_LEFT, true);
            session.button(cmvs_mobile::input::BUTTON_LEFT, false);
        }
        status = session.step(frame_ms);
        if status != Status::Running {
            break;
        }
    }
    eprintln!(
        "# {:?} after {} VM steps at {}",
        status,
        session.startup().steps(),
        session.startup().machine().current_name().escape_ascii()
    );
    if let Some(path) = shot {
        std::fs::write(&path, session.render_cpu()?.to_png())?;
        eprintln!("# wrote {path}");
    }
    session.close();
    Ok(())
}

/// Parses "x,y" in screen pixels.
fn point(text: &str) -> Option<(f64, f64)> {
    let (x, y) = text.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}
