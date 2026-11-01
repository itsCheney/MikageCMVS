//! The engine's platform requests on a phone: there is no window to resize, title
//! or decorate, so requests are recorded and answered from the host view.
use cmvs_core::{
    platform::{PlatformBackend, PlatformRequest},
    raster::RgbaImage,
};
use std::error::Error;

pub struct MobileBackend<'a> {
    /// Drawable size of the host view, or the logical screen without one.
    pub surface: [u32; 2],
    pub state: &'a mut DisplayState,
    /// Last presented logical frame, read only when the game asks (0x203e).
    pub device_frame: Option<&'a cmvs_wgpu::GpuFrame>,
}

/// Display settings the game sets and reads back.
#[derive(Debug, Clone, Copy)]
pub struct DisplayState {
    pub fullscreen: bool,
    pub aspect_mode: u32,
    pub aspect_scaling: bool,
    pub render_mode: u32,
    pub windowed_size: [u32; 2],
}

impl PlatformBackend for MobileBackend<'_> {
    fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error>> {
        self.device_frame
            .ok_or("no frame has been presented yet")?
            .read()
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.surface)
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.state.windowed_size)
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.state.fullscreen)
    }
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.state.aspect_mode)
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.state.aspect_scaling)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.state.render_mode)
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error>> {
        Ok(false)
    }
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
        Ok(0)
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        match request {
            PlatformRequest::WindowSize(size) => self.state.windowed_size = *size,
            PlatformRequest::AspectMode(mode) => {
                if *mode > 3 {
                    return Err("invalid aspect mode".into());
                }
                self.state.aspect_mode = *mode;
            }
            PlatformRequest::AspectScaling(on) => self.state.aspect_scaling = *on,
            PlatformRequest::RenderMode(mode) => {
                if *mode > 1 {
                    return Err("unsupported render mode".into());
                }
                self.state.render_mode = *mode;
            }
            PlatformRequest::Fullscreen(on) => self.state.fullscreen = *on,
            PlatformRequest::Title(_) | PlatformRequest::Icon { .. } => {}
            PlatformRequest::Cursor { id, .. } => {
                crate::log::emit(crate::log::DEBUG, &format!("cursor {id}"))
            }
        }
        Ok(())
    }
}
