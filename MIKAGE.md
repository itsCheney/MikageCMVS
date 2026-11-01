# Mikage fork notes

This is Mikage's fork of [Shizuku-in/cmvs-rs](https://github.com/Shizuku-in/cmvs-rs), used as the CMVS runtime of the Mikage iOS player. Work happens on branch `mikage`; `main` mirrors upstream. Upstream's licence (MIT OR Apache-2.0) and notices apply unchanged.

## What the fork adds

| Change | Where | Why |
|---|---|---|
| `cmvs-mobile` crate: C interface, header `platform/ios/include/cmvs_mobile.h` | `crates/cmvs-mobile` | Runs a game into a host UIView (wgpu on its CAMetalLayer), with host-registered fonts, VK-code keys, pointer, a pulled audio mixer and a log callback. Built as a static library or framework for iOS. |
| Lenient profile selection | `crates/cmvs-mobile/src/select.rs` | Upstream refuses games whose script hashes it has not recorded. The player picks the profile of a recorded engine exe in the folder, else the 2016 baseline, and allows presumed and unreviewed commands. Unsupported commands still stop, with the script, PC and command in the error. |
| `NativeAudio::with_mixer` and a `device` feature | `crates/cmvs-audio` | Lets the host pull samples from a rodio mixer instead of opening a cpal device. Default features are unchanged for desktop. |
| `<game>/savedata` accepted as the user data folder | `crates/cmvs-core/src/system_save/profile.rs` | Mikage keeps every engine's saves next to the game. Only that one direct child of the game root is allowed; the rest of the root stays read only. |
| `cmvs-mobile-run` | `crates/cmvs-mobile/src/bin` | Headless check of a session on a desktop (virtual clock, taps, CPU-drawn PNG of the last frame). |

The font code is upstream's `cmvs-desktop` `font.rs`/`font_library.rs`, compiled into `cmvs-mobile` with `#[path]`, so it follows upstream without copies.

## Building

```bash
cargo test -p cmvs-mobile
cargo check -p cmvs-mobile --target aarch64-apple-ios
cargo build -p cmvs-mobile --release --target aarch64-apple-ios   # needs Xcode to link
cargo run -p cmvs-mobile --bin cmvs-mobile-run -- <game> --font <font.ttc> --face "Hiragino Sans" --seconds 30 --shot out.png
```

## Known limits

- Shift-JIS text only (the upstream font path decodes Shift-JIS); GBK patches need more work.
- The screen is 1280x720, the size every verified build uses.
- Movies play without sound (upstream).
- Chinese patches that hook the engine with a DLL (for example 茶组's `amatsu.dll` with `Pk/*.pk` for アマツツミ) are not read.

## Syncing upstream

Merge upstream `main` into `mikage`. The fork's changes outside `crates/cmvs-mobile` are small on purpose; conflicts should stay in `crates/cmvs-audio/src/lib.rs`, `Cargo.toml` and `system_save/profile.rs`.
