//! Host input (Windows virtual-key codes and a pointer in surface pixels) mapped to
//! the CMVS input-code table, the way cmvs-desktop's `game_input` maps winit events.
use cmvs_core::input::{ActionInput, CodeInput, KeyBindings};
use cmvs_core::viewport::Viewport;
use std::collections::BTreeSet;

/// The logical screen every verified build draws.
pub const SCREEN: [u32; 2] = [1280, 720];
/// Presentation mode of the host view: the frame fitted with black bars.
pub const PRESENTATION_MODE: u32 = 1;

pub const BUTTON_LEFT: u8 = 1;
pub const BUTTON_MIDDLE: u8 = 2;
pub const BUTTON_RIGHT: u8 = 3;

/// CMVS input code (0x140056780 keyboard table) of a Windows virtual-key code.
pub fn vk_code(vk: u32) -> Option<u8> {
    Some(match vk {
        0x08 => 20,
        0x09 => 21,
        0x0D => 22,
        0x10 | 0xA0 | 0xA1 => 23,
        0x11 | 0xA2 | 0xA3 => 24,
        0x1B => 25,
        0x20 => 26,
        0x21 => 27,
        0x22 => 28,
        0x23 => 29,
        0x24 => 30,
        0x2D => 31,
        0x2E => 32,
        0x26 => 33,
        0x28 => 34,
        0x25 => 35,
        0x27 => 36,
        0x60..=0x69 => 37 + (vk - 0x60) as u8,
        0x6A => 47,
        0x6B => 48,
        0x6D => 49,
        0x6E => 50,
        0x6F => 51,
        0x70..=0x87 => 52 + (vk - 0x70) as u8,
        0x30..=0x39 => 76 + (vk - 0x30) as u8,
        0x41..=0x5A => 86 + (vk - 0x41) as u8,
        0xBA => 112,
        0xBB => 113,
        0xBC => 114,
        0xBD => 115,
        0xBE => 116,
        0xBF => 117,
        0xC0 => 118,
        0xDB => 119,
        0xDC => 120,
        0xDD => 121,
        0xDE => 122,
        _ => return None,
    })
}

#[derive(Debug, Default)]
pub struct MobileInput {
    held: BTreeSet<u8>,
    /// Codes pressed since the last frame, latched so a press and release between
    /// two frames still reads as one down frame.
    pulses: BTreeSet<u8>,
    function_pulses: u16,
    history: CodeInput,
    pointer: Option<[f64; 2]>,
    wheel: i32,
}
impl MobileInput {
    pub fn code(&mut self, code: u8, pressed: bool) {
        if pressed {
            if self.held.insert(code) {
                self.pulses.insert(code);
                if (52..=63).contains(&code) {
                    self.function_pulses |= 1 << (code - 52);
                }
            }
        } else {
            self.held.remove(&code);
        }
    }
    pub fn key(&mut self, vk: u32, pressed: bool) -> bool {
        match vk_code(vk) {
            Some(code) => {
                self.code(code, pressed);
                true
            }
            None => false,
        }
    }
    pub fn pointer(&mut self, x: f64, y: f64) {
        self.pointer = Some([x, y]);
    }
    pub fn wheel(&mut self, notches: i32) {
        self.wheel = self.wheel.saturating_add(notches);
    }
    /// Lets go of everything (the app left the foreground).
    pub fn clear(&mut self, input: &mut ActionInput) {
        self.held.clear();
        self.pulses.clear();
        self.function_pulses = 0;
        self.wheel = 0;
        self.history.clear(input);
    }
    pub fn script_position(&self, surface: [u32; 2]) -> Option<[i32; 2]> {
        let viewport = Viewport::new(SCREEN, surface, PRESENTATION_MODE).ok()?;
        viewport.to_source(self.pointer?)
    }
    pub fn frame(&mut self, bindings: &KeyBindings, surface: [u32; 2], input: &mut ActionInput) {
        let mut codes = [false; 128];
        for &code in self.held.union(&self.pulses) {
            codes[usize::from(code)] = true;
        }
        // CMVS code 4 is wheel up, 5 down.
        codes[4] = self.wheel > 0;
        codes[5] = self.wheel < 0;
        self.wheel = 0;
        input.press_function_keys(self.function_pulses);
        self.function_pulses = 0;
        self.pulses.clear();
        self.history.update(bindings, &codes, input);
        let [x, y] = self.script_position(surface).unwrap_or([-1, -1]);
        input.set_pointer(x, y);
    }
    /// The surface point of a script position (cursor warps).
    pub fn warp(&mut self, surface: [u32; 2], point: [i32; 2]) {
        if let Ok(viewport) = Viewport::new(SCREEN, surface, PRESENTATION_MODE) {
            self.pointer = Some(viewport.to_target(point));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_keys_match_the_desktop_table() {
        assert_eq!(vk_code(0x0D), Some(22));
        assert_eq!(vk_code(0xA2), Some(24));
        assert_eq!(vk_code(0x1B), Some(25));
        assert_eq!(vk_code(0x70), Some(52));
        assert_eq!(vk_code(0x7B), Some(63));
        assert_eq!(vk_code(0x5A), Some(111));
        assert_eq!(vk_code(0xDE), Some(122));
        assert_eq!(vk_code(0x07), None);
    }

    #[test]
    fn a_short_tap_reads_as_one_press_then_one_release() {
        let bindings = KeyBindings::parse(
            "[KEY_FUNCTION_01]\nKEY_DATA_01=1\n[KEY_FUNCTION_04]\nKEY_DATA_01=22",
        )
        .unwrap();
        let mut host = MobileInput::default();
        let mut input = ActionInput::default();
        host.code(BUTTON_LEFT, true);
        host.code(BUTTON_LEFT, false);
        host.key(0x0D, true);
        host.key(0x0D, false);
        host.frame(&bindings, [1280, 720], &mut input);
        assert!(input.pressed(0) && input.pressed(3));
        input.consume(0);
        host.frame(&bindings, [1280, 720], &mut input);
        assert!(input.released(0) && input.released(3));
    }

    #[test]
    fn pointer_maps_through_the_letterbox() {
        let mut host = MobileInput::default();
        // 2560x1600: the 16:9 frame is 2560x1440 with 80 px bars.
        host.pointer(1280.0, 800.0);
        assert_eq!(host.script_position([2560, 1600]), Some([640, 360]));
    }
}
