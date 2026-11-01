//! Auxiliary text content: 0x238b layout (0x14002f3e0), 0x2390 measurement (0x14002f120)
//! and the glyph half of the 0x14002edb0 frame update, which attaches glyphs inside the
//! bounds to atlas cells and detaches the others. Addresses refer to the documented binary.
use super::{
    RESEARCH_SCREEN,
    auxiliary::{AuxiliaryGlyph, AuxiliaryText},
    measure::lead,
};
use crate::FormatError;

// 、。」』 stay on the full line while +128 is set (encoding::no_break); ゛ is
// drawn a fifth of the size to the left and advances less (encoding::voiced_mark).
/// 0x14002eb10 searches cells 1..1023 of the 1024-byte table at +416.
const CELLS: u16 = 1024;
/// 0x140073c80 passes a stack local as the measured ruby-line flag array; the frame holds
/// 22 entries below its saved registers.
const MEASURE_FLAGS: u32 = 22;

/// Glyph records and the final pen of one 0x238b call, applied by `append`.
pub(super) struct TextLayout {
    pub(super) glyphs: Vec<AuxiliaryGlyph>,
    pub(super) pen: [f32; 2],
}

impl AuxiliaryText {
    fn float(&self, at: u16) -> f32 {
        f32::from_bits(self.words[&at])
    }

    /// Wrap test shared by layout and measurement: the pen is at or past the area's right
    /// edge less one glyph cell.
    fn at_edge(&self, x: f32) -> bool {
        let size = self.words[&48] as i32 as f32;
        x >= (self.float(16) + self.float(24)) - (size + self.float(52))
    }

    /// 0x14002f3e0 with a3 = -1 (the only call, from 0x140073a80). The line counter starts
    /// at zero for each call and indexes the ruby-line flags +212..+340 that the call clears
    /// first, so only the every-line flag +200 reserves the ruby height +204. Ruby text is
    /// skipped; its base characters are laid out without the voiced-mark rule.
    /// 0x1400321a0 (2017 builds, edx = 0): whether a character at pen `x` starts a new
    /// line under the 0x23b8 rules. `state` is the overflow count +0x25c and the previous
    /// character +0x260; a wrap clears both. Only a character past the edge is classified,
    /// and only while +128 is set: 1 (hanging) or 2 (inseparable and equal to the previous
    /// one) may run past the edge up to +0x198 times.
    fn rule_wrap(
        &self,
        rules: &super::LineBreakRules,
        x: f32,
        code: u16,
        state: &mut (u32, u16),
    ) -> bool {
        if !self.at_edge(x) {
            state.1 = code;
            return false;
        }
        if self.words[&128] != 0 {
            // 0x1400320d0 walks byte pairs up to the NUL terminator.
            let class = |set: &[u8]| {
                set.chunks(2)
                    .any(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]) == code)
            };
            let kind = if class(&rules.hanging) {
                1
            } else if class(&rules.inseparable) {
                2
            } else {
                0
            };
            if kind == 1 || (kind == 2 && state.1 == code) {
                state.0 = state.0.wrapping_add(1);
                if rules.max_overflow as u32 >= state.0 {
                    return false;
                }
            }
        }
        *state = (0, 0);
        true
    }

    pub(super) fn layout(&self, text: &[u8]) -> Result<TextLayout, &'static str> {
        let byte = |i: usize| text.get(i).copied().unwrap_or(0);
        let size = self.words[&48];
        let size_f = size as i32 as f32;
        let gradient = match self.words[&148] {
            value @ 1..=3 => value,
            _ => 0,
        };
        let template = AuxiliaryGlyph {
            face: self.face.clone(),
            code: 0,
            position: [0, 0],
            size,
            colors: [self.words[&132], self.words[&136]],
            gradient,
            outline: [self.words[&140], self.words[&144]],
            cell: 0,
            layer: 0,
        };
        let no_break = self.words[&128] != 0;
        let (mut x, mut y) = (self.float(176), self.float(180));
        let mut line = 0u32;
        let mut glyphs = Vec::new();
        let rules = self.line_break.as_ref();
        let mut state = (0u32, 0u16);
        let breaks = |x: f32, code: u16, state: &mut (u32, u16)| match rules {
            Some(rules) => self.rule_wrap(rules, x, code, state),
            None => self.at_edge(x) && !(no_break && crate::encoding::no_break(code)),
        };
        // 0x14002f581 / 0x14002f6c9 / 0x14002f877: a line feed after the wrap test.
        let wrap = |x: &mut f32, y: &mut f32, line: &mut u32| -> Result<(), &'static str> {
            *line += 1;
            if *line >= 32 {
                return Err("auxiliary text exceeds the 32 cleared ruby-line flags");
            }
            if self.words[&200] != 0 {
                *y += self.words[&204] as i32 as f32;
            }
            *x = self.float(16);
            *y += size_f + self.float(56);
            Ok(())
        };
        let truncate = |v: f32| {
            if (-2147483648.0..2147483648.0).contains(&v) {
                v as i32
            } else {
                i32::MIN
            }
        };
        let mut i = 0;
        while byte(i) != 0 {
            let first = byte(i);
            if first == b'\\' {
                match byte(i + 1) {
                    b'c' | b't' => i += 2,
                    b'n' => {
                        // Raw +16 copy; neither the line counter nor the ruby height moves.
                        x = self.float(16);
                        y += size_f + self.float(56);
                        i += 2;
                    }
                    b'w' => i += 3,
                    _ => return Err("unknown auxiliary text escape loops forever"),
                }
                if i > text.len() {
                    return Err("auxiliary text escape reads past the terminator");
                }
                continue;
            }
            if first == b'{' {
                if let Some(rules) = rules
                    && self.rule_wrap(rules, x, 0, &mut state)
                {
                    wrap(&mut x, &mut y, &mut line)?;
                }
                i += 1;
                loop {
                    let b = byte(i);
                    if b == 0 {
                        return Err("unterminated auxiliary ruby group");
                    }
                    if lead(b) {
                        let code = u16::from_be_bytes([b, byte(i + 1)]);
                        i += 2;
                        if breaks(x, code, &mut state) {
                            wrap(&mut x, &mut y, &mut line)?;
                        }
                        glyphs.push(AuxiliaryGlyph {
                            code,
                            position: [truncate(x), truncate(y)],
                            ..template.clone()
                        });
                        x += size_f + self.float(52);
                    } else {
                        i += 1;
                        if b == b'/' {
                            break;
                        }
                    }
                }
                loop {
                    let b = byte(i);
                    if b == 0 {
                        return Err("unterminated auxiliary ruby group");
                    }
                    if lead(b) {
                        i += 2;
                    } else {
                        i += 1;
                        if b == b'}' {
                            break;
                        }
                    }
                }
                continue;
            }
            if !lead(first) {
                // 0x14002f7a2 decides the half-width advance from a word the caller
                // never stored (r8's home slot), so the original's result is whatever
                // that slot holds. Take the measurement's rule (0x14002f120), which
                // sizes the backlog entries: wrap at the edge, advance half a cell.
                let code = first as i8 as i16 as u16;
                i += 1;
                if self.at_edge(x) {
                    wrap(&mut x, &mut y, &mut line)?;
                    state = (0, 0);
                } else if rules.is_some() {
                    state.1 = code;
                }
                glyphs.push(AuxiliaryGlyph {
                    code,
                    position: [truncate(x), truncate(y)],
                    ..template.clone()
                });
                x += size_f * 0.5 + self.float(52);
                continue;
            }
            let code = u16::from_be_bytes([first, byte(i + 1)]);
            i += 2;
            if breaks(x, code, &mut state) {
                wrap(&mut x, &mut y, &mut line)?;
            }
            let mut position = [truncate(x), truncate(y)];
            if code == crate::encoding::voiced_mark() {
                position[0] = (position[0] as u32).wrapping_sub(size / 5) as i32;
                x += (size_f * 0.5 - size_f / 5.0) + self.float(52);
            } else {
                x += size_f + self.float(52);
            }
            glyphs.push(AuxiliaryGlyph {
                code,
                position,
                ..template.clone()
            });
        }
        Ok(TextLayout {
            glyphs,
            pen: [x, y],
        })
    }

    /// Commits a layout of this group's current state.
    pub(super) fn append(&mut self, layout: TextLayout) {
        for at in (212..340).step_by(4) {
            self.words.insert(at, 0);
        }
        self.words.insert(176, layout.pen[0].to_bits());
        self.words.insert(180, layout.pen[1].to_bits());
        self.glyphs.extend(layout.glyphs);
    }

    /// 0x14002f120 from the current pen: (lines, widest line, last line), widths counted in
    /// Shift-JIS bytes. An unknown escape ends the measurement early.
    pub(super) fn measure(&self, text: &[u8]) -> Result<[u32; 3], &'static str> {
        self.measure_with(text, None).map(|(bytes, _)| bytes)
    }

    /// 0x23bb (2017 builds, 0x14007acb0 -> 0x140033970): the same walk with widths in
    /// pixels, (lines, widest line, last line). The original asks GDI for each glyph;
    /// this uses the cell advances of the layout. Unlike 0x2390 it applies the 0x23b8 rules.
    pub(super) fn measure_pixels(&self, text: &[u8]) -> Result<[u32; 3], &'static str> {
        self.measure_with(text, self.line_break.as_ref())
            .map(|(_, pixels)| pixels)
    }

    fn measure_with(
        &self,
        text: &[u8],
        rules: Option<&super::LineBreakRules>,
    ) -> Result<([u32; 3], [u32; 3]), &'static str> {
        let mut state = (0u32, 0u16);
        let byte = |i: usize| text.get(i).copied().unwrap_or(0);
        let size_f = self.words[&48] as i32 as f32;
        let no_break = self.words[&128] != 0;
        let mut x = self.float(176);
        let (mut columns, mut lines, mut widest) = (0u32, 0u32, 0u32);
        let (mut line_px, mut widest_px) = (0f32, 0f32);
        let mut i = 0;
        while byte(i) != 0 {
            let first = byte(i);
            if first == b'\\' {
                match byte(i + 1) {
                    b'c' | b't' => i += 2,
                    b'n' => {
                        x = self.float(16);
                        i += 2;
                        columns = 0;
                        line_px = 0.0;
                        lines += 1;
                    }
                    b'w' => i += 3,
                    _ => break,
                }
                if i > text.len() {
                    return Err("auxiliary text escape reads past the terminator");
                }
                continue;
            }
            if first == b'{' {
                let edge = match rules {
                    Some(rules) => self.rule_wrap(rules, x, 0, &mut state),
                    None => self.at_edge(x),
                };
                if edge {
                    x = self.float(16);
                    lines += 1;
                    columns = 0;
                    line_px = 0.0;
                }
                if lines >= MEASURE_FLAGS {
                    return Err("measured ruby line overwrites the caller's stack frame");
                }
                i += 1;
                loop {
                    let b = byte(i);
                    if b == 0 {
                        return Err("unterminated auxiliary ruby group");
                    }
                    if lead(b) {
                        columns += 2;
                        i += 2;
                        x += size_f + self.float(52);
                        line_px += size_f + self.float(52);
                        widest = widest.max(columns);
                        widest_px = widest_px.max(line_px);
                    } else {
                        i += 1;
                        if b == b'/' {
                            break;
                        }
                    }
                }
                loop {
                    let b = byte(i);
                    if b == 0 {
                        return Err("unterminated auxiliary ruby group");
                    }
                    if lead(b) {
                        i += 2;
                    } else {
                        i += 1;
                        if b == b'}' {
                            break;
                        }
                    }
                }
                continue;
            }
            if lead(first) {
                let code = u16::from_be_bytes([first, byte(i + 1)]);
                i += 2;
                let advance = size_f + self.float(52);
                let edge = match rules {
                    Some(rules) => self.rule_wrap(rules, x, code, &mut state),
                    None => {
                        x >= (self.float(16) + self.float(24)) - advance
                            && !(no_break && crate::encoding::no_break(code))
                    }
                };
                if edge {
                    columns = 0;
                    line_px = 0.0;
                    lines += 1;
                    x = self.float(16);
                }
                columns += 2;
                x += advance;
                line_px += advance;
            } else {
                i += 1;
                if x >= (self.float(16) + self.float(24)) - (self.float(52) + size_f) {
                    x = self.float(16);
                    lines += 1;
                    columns = 0;
                    line_px = 0.0;
                    state = (0, 0);
                } else if rules.is_some() {
                    state.1 = first as i8 as i16 as u16;
                }
                columns += 1;
                x += size_f * 0.5 + self.float(52);
                line_px += size_f * 0.5 + self.float(52);
            }
            widest = widest.max(columns);
            widest_px = widest_px.max(line_px);
        }
        let lines = lines.wrapping_add(1);
        Ok((
            [lines, widest, columns],
            [lines, widest_px as u32, line_px as u32],
        ))
    }

    /// Glyph half of 0x14002edb0, after the motion: in record order, a glyph whose padded
    /// cell lies outside the bounds +32..+44 loses its attached object and cell; one inside
    /// without an object gets the lowest free cell (0x14002eb10) and is appended to the
    /// atlas object's list. With no free cell it stays detached and is retried next frame.
    pub(super) fn attach_glyphs(&mut self) -> Result<(), FormatError> {
        if self.glyphs.is_empty() {
            return Ok(());
        }
        let [left, top, width, height] = [32, 36, 40, 44].map(|at| self.float(at));
        let (right, bottom) = (left + width, top + height);
        let origin = [self.float(8), self.float(12)];
        for index in 0..self.glyphs.len() {
            let glyph = &self.glyphs[index];
            let x = glyph.position[0] as f32 + origin[0];
            let y = glyph.position[1] as f32 + origin[1];
            let size = glyph.size as i32 as f32;
            let outside = x - 2.0 > right
                || left > x + size + 2.0
                || y - 2.0 > bottom
                || top > y + size + 2.0;
            // Attached glyphs always hold a cell, since the search starts at 1.
            if outside {
                if glyph.cell != 0 {
                    self.attached.retain(|&i| i != index);
                    self.cells.remove(&glyph.cell);
                    self.glyphs[index].cell = 0;
                }
            } else if glyph.cell == 0 {
                let Some(cell) = (1..CELLS).find(|c| !self.cells.contains(c)) else {
                    continue;
                };
                self.check_cell(cell)?;
                self.cells.insert(cell);
                self.glyphs[index].cell = cell;
                self.glyphs[index].layer = self.words[&172];
                self.attached.push(index);
            }
        }
        Ok(())
    }

    /// 0x14002eb10 scales the descriptor by +352 / +360 and places the cell with the atlas
    /// grid from creation, but sizes it from the current +48. Only the unscaled case with
    /// at most the creation size is modeled, and the cell must lie inside the atlas texture.
    fn check_cell(&self, cell: u16) -> Result<(), FormatError> {
        if [352, 356, 360].map(|at| self.words[&at])
            != [RESEARCH_SCREEN[0], RESEARCH_SCREEN[1], RESEARCH_SCREEN[0]]
        {
            return Err(FormatError("scaled auxiliary glyphs are not verified"));
        }
        let side =
            (self.words[&352].wrapping_mul(self.words[&48]) / self.words[&360]).wrapping_add(6);
        // アオイトリ's backlog creates its group at size 30 and draws at 28: a
        // smaller glyph stays inside its cell. A larger one would spill into
        // the neighbouring cells, which is not modeled.
        if side > self.atlas.cell_side {
            return Err(FormatError(
                "auxiliary glyph size exceeds the atlas cell size",
            ));
        }
        let pages = self.atlas.texture_size[1] / self.atlas.texture_size[0];
        if u32::from(cell) / self.atlas.cells_per_page >= pages {
            return Err(FormatError(
                "auxiliary glyph cell lies outside the atlas texture",
            ));
        }
        Ok(())
    }

    /// Placement of the atlas object for the backend: its mode-5 draw state takes the float
    /// position (0x140020d3d) and attached glyphs inherit it, together with the second
    /// texture while +88 is 4 (0x14003b420). Only integral positions at unit scale are
    /// supported.
    pub fn glyph_origin(&self) -> Result<[i32; 2], FormatError> {
        let draw = self.object.draw.as_ref().expect("owned atlas draw state");
        if draw.words[18..21] != [1.0f32.to_bits(), 1.0f32.to_bits(), 0] || draw.words[17] != 255 {
            return Err(FormatError(
                "auxiliary atlas transform or opacity is not verified",
            ));
        }
        let position = [draw.words[11], draw.words[12]].map(f32::from_bits);
        if position
            .iter()
            .any(|v| v.fract() != 0.0 || !(-2147483648.0..2147483648.0).contains(v))
        {
            return Err(FormatError(
                "auxiliary atlas position is not an integral pixel",
            ));
        }
        Ok(position.map(|v| v as i32))
    }
}
