//! Two switches of the 2017 builds (cmvs-20171129) that scripts set once at start-up.
//! Addresses refer to the アオイトリ cmvs64.exe (timestamp 0x5a1e5b72); the older
//! profiles stop both commands at their gate.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineFlags {
    /// Sound device +0xaec (constructor 0x14008e290 sets 1). While it and +0xae8 are
    /// clear, the mixer forces the volumes down when the window is inactive.
    pub background_sound: bool,
    /// Global 0x14010accc: the glyph lookup 0x140005910 uses the replacement table that
    /// 0x23c0..0x23c2 fill. With an empty table the flag changes nothing.
    pub glyph_replacement: bool,
}

impl Default for EngineFlags {
    fn default() -> Self {
        Self {
            background_sound: true,
            glyph_replacement: false,
        }
    }
}

impl EngineFlags {
    /// 0x22ed (0x1400834c0, pops 8): a zero SP-4 copies +0xaec to system result 0,
    /// otherwise +0xaec = bool(SP-8). 0x23ac (0x140086f70, pops 4): the replacement flag
    /// = bool(SP-4), no result.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        match vm.pending_system() {
            Some((pc, 0x3ac)) => {
                let value = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x23ac, e.0))?;
                vm.complete_system(SystemResult(0x4004))?;
                self.glyph_replacement = value != 0;
                Ok(true)
            }
            Some((pc, 0x2ed)) => {
                let arg = |distance| {
                    vm.stack()
                        .peek_u32(distance)
                        .map_err(|e| error(pc, 0x22ed, e.0))
                };
                let value = if arg(4)? != 0 {
                    Some(arg(8)? != 0)
                } else {
                    None
                };
                vm.complete_system(SystemResult(0x4008))?;
                match value {
                    Some(on) => self.background_sound = on,
                    None => vm
                        .state_mut()
                        .set_system_integer(0, u32::from(self.background_sound))
                        .expect("slot 0 exists"),
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
