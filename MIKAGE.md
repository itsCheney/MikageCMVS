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
| Translation patches (`PatchFiles`) | `crates/cmvs-core/src/patch.rs`, `crates/cmvs-mobile/src/patch.rs` | Scripts, images and movies of a patch are served before the game's own, as the patch's hooking DLL does. `cmvs-mobile` reads the `Pk/*.pk` format of 茶组's アマツツミ patch: raw-DEFLATE entries holding unobfuscated PS2A scripts and PNGs named after the PB3 they replace, plus `Pk/*.cmv` movies. With a patch present the session switches to GBK (`CMVS_ENCODING_AUTOMATIC`). |
| Half-width text in auxiliary text (backlog) | `crates/cmvs-core/src/message/auxiliary_layout.rs` | Upstream stopped with an error: the original reads an uninitialized stack word there. Laid out like the measurement does instead (half a cell, wrap at the edge), so backlog entries with ASCII no longer stop the game. |
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
- Only the `Pk/*.pk` patch layout is read. Other DLL patches need their own `PatchFiles` reader.
- From upstream's アマツツミ notes: 0x22a2/0x22a8/0x22ae (named effect objects) and 0x2328 (line-split wave effect) have no handler, and 3D view mode 1 is not drawn. None of them was reached on the recorded routes.

## Syncing upstream

Merge upstream `main` into `mikage`. The fork's changes outside `crates/cmvs-mobile` are small on purpose; conflicts should stay in `crates/cmvs-audio/src/lib.rs`, `Cargo.toml` and `system_save/profile.rs`.

## Tested

アマツツミ v1.03 (not recorded upstream, so the cmvs32.exe profile is used), headless with `cmvs-mobile-run`:

- Japanese: title, quit dialog, load screen, the opening story (text, CG, menu bar, BGM/SE through the mixer) and the OP movie with its soundtrack.
- 茶组 Chinese patch (`Pk/Scripts.pk`, `Graphic.pk`, `op.cmv` next to the game): Chinese title, story text, backlog and the subtitled OP.

Not yet run on a device.
