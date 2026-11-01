# Mikage fork notes

This is Mikage's fork of [Shizuku-in/cmvs-rs](https://github.com/Shizuku-in/cmvs-rs), used as the CMVS runtime of the Mikage iOS player. Work happens on branch `mikage`; `main` mirrors upstream. Upstream's licence (MIT OR Apache-2.0) and notices apply unchanged.

## What the fork adds

| Change | Where | Why |
|---|---|---|
| `cmvs-mobile` crate: C interface, header `platform/ios/include/cmvs_mobile.h` | `crates/cmvs-mobile` | Runs a game into a host UIView (wgpu on its CAMetalLayer), with host-registered fonts, VK-code keys, pointer, a pulled audio mixer and a log callback. Built as a static library or framework for iOS. |
| Lenient profile selection | `crates/cmvs-mobile/src/select.rs` | Upstream refuses games whose script hashes it has not recorded. The player picks the profile of a recorded engine exe in the folder, else the 2016 baseline, and allows presumed and unreviewed commands. Unsupported commands still stop, with the script, PC and command in the error. |
| `NativeAudio::with_mixer` and a `device` feature | `crates/cmvs-audio` | Lets the host pull samples from a rodio mixer instead of opening a cpal device. Default features are unchanged for desktop. |
| `<game>/savedata` accepted as the user data folder | `crates/cmvs-core/src/system_save/profile.rs` | Mikage keeps every engine's saves next to the game. Only that one direct child of the game root is allowed; the rest of the root stays read only. |
| GBK text | `crates/cmvs-core/src/encoding.rs`, desktop `font.rs`/`font_library.rs` | Lead bytes, the no-break punctuation and ゛ follow a per-thread text encoding (Shift-JIS by default), and glyph codes and face names decode with it. Chinese patches hook exactly these in the engine. |
| Translation patches (`PatchFiles`) | `crates/cmvs-core/src/patch.rs`, `crates/cmvs-mobile/src/patch.rs` | Scripts, images and movies of a patch are served before the game's own, as the patch's hooking DLL does. `cmvs-mobile` reads three kinds and layers them, first wins. A `patch/` folder of loose files named like the game's (any subfolder; scripts `*.ps3` encrypted or decoded, images `*.pb3` or a PNG named after one, movies `*.cmv`): anything unpacked by hand or with GARbro works this way. PKP archives (茶组's アマツツミ patch ships them as `Pk/*.pk`): raw-DEFLATE entries holding unobfuscated PS2A scripts and PNGs named after the PB3 they replace, with `.cmv` movies beside them. `Longinus.dat` (the Longinus hook, アオイトリ's 天之圣杯 patch): files keyed by a BKDR-131 hash of the upper-case UTF-16 name, holding decoded PS2A scripts and BGRA pixels laid over the game's PB3 at its size; the game starts from `data/pack/start.CHS`, as the patched exe does. Archives are found by their first bytes in the game folder or one folder down, whatever their names. With any patch the session switches to GBK (`CMVS_ENCODING_AUTOMATIC`). |
| 2017 engine build (`cmvs-20171129`) | `crates/cmvs-core/profiles`, `profile.rs`, `dispatch.rs`, `audio.rs`, `message.rs`, `message/auxiliary_layout.rs` | アオイトリ's Mikage-tested exe (2017-11-29) as a profile, chosen by exe hash. Upstream since added the 2017-11-14 build of the same game (`cmvs-20171114`) with IDA-checked font-cell text (0x211d measurement, 0x23ac cell advance, `message/font_cells.rs`); the 20171129 profile shares those semantics and bank sizes. The fork adds what upstream does not have: 0x22ed keep sound while inactive (audio host), and 0x23b8 kinsoku rules / 0x23bb pixel measurement for an auxiliary text group. Saves use the base's text v3 layout. |
| Source rectangles past the image edge | `crates/cmvs-core/src/scene_render.rs` | アオイトリ's menu reads a strip that runs 6 rows past its sheet. The part outside draws nothing instead of stopping the game. |
| Auxiliary glyphs smaller than their atlas cell | `crates/cmvs-core/src/message/auxiliary_layout.rs` | アオイトリ's backlog creates its text group at size 30 and draws at 28. A smaller glyph fits its cell; only a larger one still stops. |
| Half-width text in auxiliary text (backlog) | `crates/cmvs-core/src/message/auxiliary_layout.rs` | Upstream stopped with an error: the original reads an uninitialized stack word there. Laid out like the measurement does instead (half a cell, wrap at the edge), so backlog entries with ASCII no longer stop the game. |
| Ruby in proportional font-cell text | `crates/cmvs-core/src/message/font_cells.rs` | Upstream stops there (not yet compared with the original). アオイトリ uses it in its opening; the ruby is spread over the proportional base width as for fixed cells. |
| Host fonts up to 128 MB, with fallback | `crates/cmvs-mobile/src/session.rs`, desktop `font.rs`/`font_library.rs` | The PingFang face the iOS app rebuilds as TrueType is over the desktop's 32 MB limit and was cut short. If the first registered font still fails, the next one that loads becomes the default and the failure is logged. |
| `cmvs-mobile-run` | `crates/cmvs-mobile/src/bin` | Headless check of a session on a desktop (virtual clock, taps, wheel, sound pulled in step with the clock, CPU-drawn PNG of the last frame). |

The font code is upstream's `cmvs-desktop` `font.rs`/`font_library.rs`, compiled into `cmvs-mobile` with `#[path]`, so it follows upstream without copies.

## Building

```bash
cargo test -p cmvs-mobile
cargo check -p cmvs-mobile --target aarch64-apple-ios
cargo build -p cmvs-mobile --release --target aarch64-apple-ios   # needs Xcode to link
cargo run -p cmvs-mobile --bin cmvs-mobile-run -- <game> --font <font.ttc> --face "Hiragino Sans" --seconds 120 \
    --click 20000:640,275 --tap-every 1500 --tap-at 1150,120 --shot out.png   # click 「はじめから」 on アマツツミ, then advance text
```

## Known limits

- The screen is 1280x720, the size every verified build uses.
- Movies follow the soundtrack's position, so the host must render the session's audio for them to advance.
- Patch archives other than PKP and Longinus need their own `PatchFiles` reader, or extracting to the `patch/` folder. Patches that change the engine's code (fonts, layout) rather than its files can't be reproduced this way.
- The 20171129 profile's handlers are unreviewed for that exact exe, so they run only under the player's lenient policy. Under the font-cell semantics 0x238b uses upstream's own auxiliary layout, so the 0x23b8 rules only affect 0x23bb. 0x23c0..0x23c2 (glyph replacement table) and 0x20e4/0x20ef/0x23ae/0x23b9/0x23ba appear in no script of アオイトリ and stop if reached.
- From upstream's アマツツミ notes: 0x22a2/0x22a8/0x22ae (named effect objects) and 0x2328 (line-split wave effect) have no handler, and 3D view mode 1 is not drawn. None of them was reached on the recorded routes.

## Syncing upstream

Merge upstream `main` into `mikage` (last merged: upstream f9ea239, 2026-11-01). The fork's changes outside `crates/cmvs-mobile` are small on purpose; conflicts should stay in `crates/cmvs-audio/src/lib.rs`, `Cargo.toml`, `system_save/profile.rs`, `profile.rs`, `dispatch.rs`, `runner.rs`, `message.rs` and `message/measure.rs`. Fork commands are `ROOT` entries in `dispatch.rs` marked `Mikage:`, handled by the host that owns the state; tests that count a capability's entries (`tests/dispatch_audio.rs`) list them too.

## Tested

アマツツミ v1.03 (not recorded upstream, so the cmvs32.exe profile is used), headless with `cmvs-mobile-run`:

- Japanese: title, quit dialog, load screen, the opening story (text, CG, menu bar, BGM/SE through the mixer) and the OP movie with its soundtrack.
- 茶组 Chinese patch (`Pk/Scripts.pk`, `Graphic.pk`, `op.cmv` next to the game): Chinese title, story text, backlog and the subtitled OP.

アオイトリ (2017-11-29 build, not recorded upstream), headless:

- Japanese: logo, title, system settings, the opening story with voices and the backlog. Probe: S1, S2, S4, R1, D1, V1, V2, I3, I4 pass (S3 lists the unreviewed commands; I1 calls the backlog's buttons an ambiguous choice without a manifest).
- 天之圣杯 Chinese patch (`Longinus.dat`, `data/pack/start.CHS`): translated title and menus, GBK story text in the balloons. Probe: S1, S2, R1, I4, V1, V2 pass.

Not yet run on a device.
