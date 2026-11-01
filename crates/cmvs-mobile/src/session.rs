//! One running game: the engine's `Startup` plus the host-side services cmvs-desktop
//! provides on a PC (fonts, input, audio, GPU presentation), without a window system.
use crate::{
    audio,
    backend::{DisplayState, MobileBackend},
    font::FontRenderer,
    font_library::{FontLibrary, family_names},
    input::{MobileInput, SCREEN},
    log, patch,
    select::{self, Basis},
};
use cmvs_core::{
    encoding::TextEncoding,
    input::KeyBindings,
    message::MESSAGE_WINDOWS,
    ps3::Script,
    raster::RgbaImage,
    runner::{SaveTime, Startup, StartupEvent, load_start},
    scene_render::prepare_scene,
    settings::DEFAULT_FONT_FACE,
};
use cmvs_wgpu::{GpuFrame, Presenter, Renderer, wgpu};
use rodio::mixer::MixerSource;
use std::{
    error::Error,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// VM time per `step` before the frame is handed back to the host. The VM keeps
/// going on the next step; effects advance and frames draw only at its yields.
const STEP_BUDGET: Duration = Duration::from_millis(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// GBK when the game has a Chinese patch (`Pk/` or `Longinus.dat`), else Shift-JIS.
    Automatic,
    ShiftJis,
    Gbk,
}

/// A font file and the face in it: an index, or a family name to look up.
#[derive(Debug, Clone)]
pub struct FontFile {
    pub path: PathBuf,
    pub face: Option<String>,
}

pub struct Config {
    pub game_dir: PathBuf,
    /// Saves and settings; `<game>/savedata` when None.
    pub save_dir: Option<PathBuf>,
    pub encoding: Encoding,
    /// Local time offset for the dates written into save slots.
    pub utc_offset_ms: i64,
    /// The first font draws the game's default face; the folders of all of them are
    /// searched for faces the game names.
    pub fonts: Vec<FontFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Running,
    /// The game quit (0x2001).
    Ended,
    Failed(String),
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    renderer: Renderer,
    presenter: Option<Presenter>,
    /// Identity of the host view the presenter draws into.
    view: usize,
}

pub struct Session {
    // Fields drop in order: the engine (and its audio channels) before the GPU.
    startup: Startup<'static>,
    root: PathBuf,
    encoding: TextEncoding,
    bindings: KeyBindings,
    input: MobileInput,
    display: DisplayState,
    font: FontRenderer,
    gpu: Option<Gpu>,
    device_frame: Option<GpuFrame>,
    surface: Option<[u32; 2]>,
    audio_source: Option<MixerSource>,
    pending_ms: u32,
    /// The VM yielded since the last partial budget slice: the scene is complete.
    frame_ready: bool,
    input_due: bool,
    status: Status,
}

impl Session {
    pub fn open(config: Config) -> Result<Self> {
        let root = config.game_dir.canonicalize()?;
        let patch = patch::find(&root)?;
        let encoding = match config.encoding {
            Encoding::ShiftJis => TextEncoding::ShiftJis,
            Encoding::Gbk => TextEncoding::Gbk,
            Encoding::Automatic if patch.is_some() => TextEncoding::Gbk,
            Encoding::Automatic => TextEncoding::ShiftJis,
        };
        // Text and font decoding read it on the thread that runs the game.
        cmvs_core::encoding::set(encoding);
        if let Some((_, line)) = &patch {
            log::emit(
                log::INFO,
                &format!("translation patch: {line}, text {encoding:?}"),
            );
        }
        let patch = patch.map(|(patch, _)| patch);
        let font = font_renderer(&config.fonts)?;
        let selection = select::select(&root)?;
        for line in selection.game.selection.log_lines() {
            log::emit(log::INFO, line.trim_start_matches("# "));
        }
        match &selection.basis {
            Basis::Recorded => {}
            Basis::Executable(exe) => log::emit(
                log::WARN,
                &format!(
                    "game data is not recorded upstream; using the profile of {exe} and \
                     running presumed and unreviewed commands ({})",
                    selection.strict_error.as_deref().unwrap_or("")
                ),
            ),
            Basis::Assumed => log::emit(
                log::WARN,
                &format!(
                    "no recorded game or engine build found; assuming profile {} and \
                     running presumed and unreviewed commands ({})",
                    select::BASELINE_PROFILE,
                    selection.strict_error.as_deref().unwrap_or("")
                ),
            ),
        }
        // `Startup` borrows the start script for its whole life. A session lives
        // about as long as the app, and a start script is a few kilobytes, so it is
        // leaked rather than made self-referential.
        let start = match patch
            .as_ref()
            .map(|p| p.script(b"start.ps3"))
            .transpose()?
            .flatten()
        {
            Some(raw) => Script::decode_patch(&raw)?,
            None => load_start(&root, &selection.game.files)?,
        };
        let script: &'static _ = Box::leak(Box::new(start));
        let mut startup = Startup::new(script, &root, &selection.game, 0)?;
        startup.set_patch(patch);
        if selection.lenient() {
            startup.set_allow_presumed(true);
            startup.set_allow_unreviewed(true);
        }
        let save_dir = config
            .save_dir
            .clone()
            .unwrap_or_else(|| root.join(cmvs_core::system_save::HOST_SAVE_FOLDER));
        startup.set_user_data_directory(&save_dir)?;
        startup.set_decode_movie_frames(true);
        let (mixer, audio_source) = audio::mixer();
        startup.set_audio_backend(Box::new(cmvs_audio::NativeAudio::with_mixer(&root, mixer)?))?;
        let offset = config.utc_offset_ms;
        startup.set_save_clock(Box::new(move || local_time(offset)));
        startup.set_random_seed(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos()),
        );
        startup.set_record_text(true);
        startup.set_glyph_metrics(font.metrics());
        // 0x14000b666: a configured face that cannot be found becomes the default.
        if !font.has_face(startup.settings().font_face()) {
            startup.set_font_face(DEFAULT_FONT_FACE)?;
        }
        let bindings = read_bindings(&root)?;
        let prefs = startup.display_preferences();
        Ok(Self {
            startup,
            root,
            encoding,
            bindings,
            input: MobileInput::default(),
            display: DisplayState {
                fullscreen: prefs.fullscreen,
                aspect_mode: prefs.aspect_mode,
                aspect_scaling: prefs.aspect_scaling,
                render_mode: prefs.render_mode.min(1),
                windowed_size: SCREEN,
            },
            font,
            gpu: None,
            device_frame: None,
            surface: None,
            audio_source: Some(audio_source),
            pending_ms: 0,
            frame_ready: false,
            input_due: true,
            status: Status::Running,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn startup(&self) -> &Startup<'static> {
        &self.startup
    }
    /// The mixer's output, once; the host plays it through an [`audio::AudioOutput`].
    pub fn take_audio_source(&mut self) -> Option<MixerSource> {
        self.audio_source.take()
    }

    /// Draw into a surface made from `instance` for the host view `view`.
    pub fn attach(
        &mut self,
        view: usize,
        size: [u32; 2],
        make_surface: impl FnOnce(&wgpu::Instance) -> Result<wgpu::Surface<'static>>,
    ) -> Result<()> {
        if size.contains(&0) {
            return Err("empty surface size".into());
        }
        self.surface = Some(size);
        if self
            .gpu
            .as_ref()
            .is_some_and(|g| g.view == view && g.presenter.is_some())
        {
            return Ok(());
        }
        match &mut self.gpu {
            None => {
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::PRIMARY,
                    ..wgpu::InstanceDescriptor::new_without_display_handle()
                });
                let surface = make_surface(&instance)?;
                let (renderer, adapter) = Renderer::new(&instance, Some(&surface))?;
                log::emit(log::INFO, &format!("GPU {:?}", adapter.get_info().name));
                let presenter = Presenter::new(surface, &adapter, &renderer)?;
                self.gpu = Some(Gpu {
                    instance,
                    adapter,
                    renderer,
                    presenter: Some(presenter),
                    view,
                });
            }
            Some(gpu) => {
                gpu.presenter = None;
                let surface = make_surface(&gpu.instance)?;
                gpu.presenter = Some(Presenter::new(surface, &gpu.adapter, &gpu.renderer)?);
                gpu.view = view;
            }
        }
        Ok(())
    }
    pub fn detach(&mut self) {
        if let Some(gpu) = &mut self.gpu {
            gpu.presenter = None;
        }
        self.surface = None;
    }

    pub fn pointer(&mut self, x: f64, y: f64) {
        self.input.pointer(x, y);
    }
    pub fn button(&mut self, code: u8, down: bool) {
        self.input.code(code, down);
    }
    pub fn wheel(&mut self, notches: i32) {
        self.input.wheel(notches);
    }
    pub fn key(&mut self, vk: u32, down: bool) {
        if !self.input.key(vk, down) {
            log::emit(
                log::DEBUG,
                &format!("virtual key 0x{vk:02x} has no CMVS code"),
            );
        }
    }
    /// The app left (false) or regained (true) the foreground.
    pub fn set_focused(&mut self, focused: bool) {
        if !focused {
            self.input.clear(self.startup.input_mut());
        }
        self.startup.set_focused(focused);
        self.input_due = true;
    }

    fn surface_size(&self) -> [u32; 2] {
        self.surface.unwrap_or(SCREEN)
    }

    fn fail(&mut self, message: String) -> Status {
        let vm = self.startup.machine();
        let message = format!(
            "{message} (at {} / 0x{:x} after {} steps)",
            vm.current_name().escape_ascii(),
            vm.pc(),
            self.startup.steps()
        );
        log::emit(log::ERROR, &message);
        for line in self.startup.gate().diagnostic_lines() {
            log::emit(log::INFO, line.trim_start_matches("# "));
        }
        self.status = Status::Failed(message);
        self.status.clone()
    }

    /// Runs the game for up to one step budget after `dt_ms` of wall time, then
    /// advances effects and draws when the VM reached a frame boundary.
    pub fn step(&mut self, dt_ms: u32) -> Status {
        cmvs_core::encoding::set(self.encoding);
        if self.status != Status::Running {
            return self.status.clone();
        }
        self.pending_ms = self.pending_ms.saturating_add(dt_ms);
        let surface = self.surface_size();
        if self.input_due {
            self.input
                .frame(&self.bindings, surface, self.startup.input_mut());
            self.input_due = false;
        }
        let mut boundary = false;
        let start = Instant::now();
        while start.elapsed() < STEP_BUDGET {
            let mut backend = MobileBackend {
                surface,
                state: &mut self.display,
                device_frame: self.device_frame.as_ref(),
            };
            let event = match self.startup.tick(&mut backend) {
                Ok(event) => event,
                Err(e) => return self.fail(e.to_string()),
            };
            for command in self.startup.take_unreviewed_notices() {
                log::emit(
                    log::WARN,
                    &format!(
                        "unreviewed system command 0x{:04x} runs with baseline semantics",
                        0x2000 + command
                    ),
                );
            }
            if event != StartupEvent::Inactive {
                self.frame_ready = event == StartupEvent::Yield;
            }
            match event {
                StartupEvent::Quit => {
                    log::emit(log::INFO, "the game quit");
                    self.status = Status::Ended;
                    return self.status.clone();
                }
                StartupEvent::Yield => {
                    self.input_due = true;
                    boundary = true;
                    break;
                }
                StartupEvent::Inactive => {
                    boundary = true;
                    break;
                }
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::MainEnteredWithoutSnapshot => log::emit(
                    log::WARN,
                    "main entered without a save snapshot: saving is off",
                ),
                StartupEvent::Unhandled { command, .. } => {
                    return self.fail(format!("unsupported system command 0x{command:04x}"));
                }
                StartupEvent::MainPreparedNeedsSnapshot => {
                    return self.fail("the save snapshot could not be built".into());
                }
            }
        }
        if let Some(point) = self.startup.input_mut().take_warp() {
            self.input.warp(surface, point);
        }
        if !boundary {
            // Mid-run: keep the previous frame, continue the VM next step.
            return Status::Running;
        }
        let dt = std::mem::take(&mut self.pending_ms);
        if let Err(e) = self.startup.advance_decoded_frame(dt) {
            return self.fail(e.to_string());
        }
        if self.frame_ready
            && let Err(e) = self.present()
        {
            return self.fail(format!("drawing: {e}"));
        }
        Status::Running
    }

    fn present(&mut self) -> Result<()> {
        let Some(size) = self.surface else {
            return Ok(());
        };
        let linear = self.display.render_mode == 1;
        let Some(gpu) = self.gpu.as_mut() else {
            return Ok(());
        };
        let Some(presenter) = gpu.presenter.as_mut() else {
            return Ok(());
        };
        let frame = if let Some(image) = self.startup.movie_frame() {
            gpu.renderer.render_image(image, SCREEN, linear)?
        } else {
            let objects = self.font.prepare_objects(self.startup.messages())?;
            let mut additional: Vec<_> = objects.iter().collect();
            additional.extend(
                (0..MESSAGE_WINDOWS)
                    .filter_map(|i| self.startup.messages().window(i))
                    .filter(|w| w.visible && !w.hidden)
                    .map(|w| &w.object),
            );
            let scene = prepare_scene(
                self.startup.scene(),
                Some(self.startup.camera()),
                &additional,
                SCREEN[0],
                SCREEN[1],
            )?;
            gpu.renderer.render(&scene)?
        };
        if presenter.lost {
            return Err("the drawing surface was lost; attach it again".into());
        }
        if presenter.present(
            &gpu.renderer,
            &frame,
            size,
            crate::input::PRESENTATION_MODE,
            linear,
            false,
        )? {
            self.device_frame = Some(frame);
        }
        Ok(())
    }

    /// The current frame drawn on the CPU (tests and thumbnails; no GPU needed).
    pub fn render_cpu(&mut self) -> Result<RgbaImage> {
        cmvs_core::encoding::set(self.encoding);
        if let Some(image) = self.startup.movie_frame() {
            return Ok(image.clone());
        }
        Ok(self.font.render(
            self.startup.scene(),
            self.startup.camera(),
            self.startup.messages(),
        )?)
    }

    /// Saves the settings the game keeps outside its system save.
    pub fn close(mut self) {
        let display = cmvs_core::settings::DisplayPreferences {
            fullscreen: self.display.fullscreen,
            aspect_mode: self.display.aspect_mode,
            aspect_scaling: self.display.aspect_scaling,
            render_mode: self.display.render_mode,
        };
        if let Err(e) = self.startup.save_preferences(display) {
            log::emit(log::WARN, &format!("settings not saved: {e}"));
        }
    }
}

/// Host fonts can be large: the PingFang the app rebuilds as TrueType from the
/// system's outlines is over 32 MB, the limit the desktop font code uses.
const MAX_FONT: usize = 128 * 1024 * 1024;

fn font_renderer(fonts: &[FontFile]) -> Result<FontRenderer> {
    if fonts.is_empty() {
        return Err("register a font before opening a game".into());
    }
    let mut directories = Vec::new();
    for font in fonts {
        if let Some(dir) = font.path.parent()
            && !directories.iter().any(|d: &PathBuf| d == dir)
        {
            directories.push(dir.to_owned());
        }
    }
    // The first registered font that loads is the default; a broken one is
    // logged and skipped so a fallback font still lets the game start.
    let mut errors: Vec<String> = Vec::new();
    for font in fonts {
        match load_font(font) {
            Ok(default) => {
                for error in &errors {
                    log::emit(log::WARN, error);
                }
                return Ok(FontRenderer::with_library(FontLibrary::with_directories(
                    Arc::new(default),
                    directories,
                )));
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    Err(errors.join("; ").into())
}

fn load_font(font: &FontFile) -> Result<ab_glyph::FontVec> {
    let index = face_index(font)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&font.path)?
        .take(MAX_FONT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FONT {
        return Err(format!(
            "{} is larger than {} MB",
            font.path.display(),
            MAX_FONT >> 20
        )
        .into());
    }
    let size = bytes.len();
    ab_glyph::FontVec::try_from_vec_and_index(bytes, index).map_err(|e| {
        format!(
            "{} is not a usable font ({size} bytes, face {index}): {e}",
            font.path.display()
        )
        .into()
    })
}

/// The collection index of a face given by name (family names, any language), or
/// the index itself when the name is a number; 0 when no face is named.
fn face_index(font: &FontFile) -> Result<u32> {
    let Some(face) = font.face.as_deref().filter(|f| !f.is_empty()) else {
        return Ok(0);
    };
    if let Ok(index) = face.parse() {
        return Ok(index);
    }
    let wanted = face.to_lowercase();
    family_names(&font.path)?
        .into_iter()
        .find(|(_, names)| names.iter().any(|n| n.to_lowercase() == wanted))
        .map(|(index, _)| index)
        .ok_or_else(|| format!("{} has no face named {face}", font.path.display()).into())
}

fn read_bindings(root: &Path) -> Result<KeyBindings> {
    let path = root.join("key.cfg");
    if !path.is_file() {
        log::emit(log::WARN, "no key.cfg: keys and buttons do nothing");
        return Ok(KeyBindings::default());
    }
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(65537)
        .read_to_string(&mut text)?;
    Ok(KeyBindings::parse(&text)?)
}

/// Wall-clock date and time shifted by `offset_ms` (proleptic Gregorian calendar).
fn local_time(offset_ms: i64) -> SaveTime {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    civil(now_ms.saturating_add(offset_ms).div_euclid(1000))
}

fn civil(seconds: i64) -> SaveTime {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [
        year as u16,
        month as u16,
        day as u16,
        (rest / 3600) as u16,
        (rest / 60 % 60) as u16,
        (rest % 60) as u16,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), [1970, 1, 1, 0, 0, 0]);
        // 2026-11-01 15:04:05 UTC.
        assert_eq!(civil(1_793_545_445), [2026, 11, 1, 15, 4, 5]);
        assert_eq!(civil(-1), [1969, 12, 31, 23, 59, 59]);
    }
}
