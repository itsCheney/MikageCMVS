//! Translation patches that hook the engine with a DLL and keep their files
//! in `Pk/*.pk` (茶组's アマツツミ Chinese patch: `amatsu.dll`).
//!
//! A `.pk` file is `PKP\0`, a zero word, the entry count, then 0xb0-byte
//! entries: data offset, unpacked size, packed size, 36 zero bytes and a
//! NUL-padded UTF-16 name. Data is raw DEFLATE. Scripts are PS2A without the
//! byte obfuscation; images are PNG files named like the PB3 they replace
//! (`cg_bg.png` for `cg_bg.pb3`). Subtitled movies sit next to the archives
//! as plain `.cmv` files. The patched scripts carry GBK text.
use cmvs_core::{patch::PatchFiles, raster::RgbaImage};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const ENTRY_SIZE: usize = 0xb0;
const NAME_OFFSET: usize = 0x30;
/// Entry and size bounds: the largest file in the known patch is 5 MB.
const MAX_ENTRIES: u32 = 65536;
const MAX_FILE: u32 = 256 << 20;

struct Entry {
    archive: usize,
    offset: u64,
    size: u32,
    packed: u32,
}

/// The `Pk` folder of a game.
pub struct PkPatch {
    archives: Vec<PathBuf>,
    /// Lower-case names; a later archive (by file name) wins.
    entries: BTreeMap<String, Entry>,
    movies: bool,
}

impl PkPatch {
    /// The patch in `<root>/Pk`, or None when the game has none.
    pub fn find(root: &Path) -> Result<Option<Self>> {
        let Some(dir) = child_ignoring_case(root, "pk")? else {
            return Ok(None);
        };
        let mut archives = Vec::new();
        let mut movies = false;
        for item in std::fs::read_dir(&dir)? {
            let path = item?.path();
            match extension(&path).as_deref() {
                Some("pk") => archives.push(path),
                Some("cmv") => movies = true,
                _ => {}
            }
        }
        if archives.is_empty() {
            return Ok(None);
        }
        archives.sort();
        let mut entries = BTreeMap::new();
        for (archive, path) in archives.iter().enumerate() {
            for (name, entry) in
                read_index(path, archive).map_err(|e| format!("{}: {e}", path.display()))?
            {
                entries.insert(name, entry);
            }
        }
        Ok(Some(Self {
            archives,
            entries,
            movies,
        }))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.entries.get(&name.to_ascii_lowercase()) else {
            return Ok(None);
        };
        let mut file = File::open(&self.archives[entry.archive])?;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut packed = vec![0; entry.packed as usize];
        file.read_exact(&mut packed)?;
        let data = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, entry.size as usize)
            .map_err(|e| format!("patch file {name}: bad DEFLATE data ({e})"))?;
        if data.len() != entry.size as usize {
            return Err(format!("patch file {name}: size differs from its entry").into());
        }
        Ok(Some(data))
    }
}

impl PatchFiles for PkPatch {
    fn script(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        match std::str::from_utf8(name) {
            Ok(name) => self.read(name),
            Err(_) => Ok(None),
        }
    }

    fn image(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        let Ok(name) = std::str::from_utf8(name) else {
            return Ok(None);
        };
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        let Some(png) = self.read(&format!("{stem}.png"))? else {
            return Ok(None);
        };
        let (image, alpha) =
            decode_png(&png).map_err(|e| format!("patch image {stem}.png: {e}"))?;
        Ok(Some(cmvs_core::patch::raw_image(&image, alpha)))
    }

    fn movie_directory(&self) -> Option<&[u8]> {
        self.movies.then_some(b"Pk\\".as_slice())
    }
}

fn read_index(path: &Path, archive: usize) -> Result<Vec<(String, Entry)>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut head = [0u8; 12];
    file.read_exact(&mut head)?;
    if &head[..4] != b"PKP\0" {
        return Err("not a PKP archive".into());
    }
    let count = u32::from_le_bytes(head[8..12].try_into().expect("4 bytes"));
    if count > MAX_ENTRIES {
        return Err("too many entries".into());
    }
    let mut table = vec![0u8; count as usize * ENTRY_SIZE];
    file.read_exact(&mut table)?;
    let mut out = Vec::new();
    for raw in table.chunks_exact(ENTRY_SIZE) {
        let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().expect("4 bytes"));
        let (offset, size, packed) = (word(0), word(4), word(8));
        if size > MAX_FILE || u64::from(offset) + u64::from(packed) > length {
            return Err("entry outside the archive".into());
        }
        let units: Vec<u16> = raw[NAME_OFFSET..]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let name = String::from_utf16(&units).map_err(|_| "entry name is not UTF-16")?;
        if name.is_empty() {
            return Err("empty entry name".into());
        }
        out.push((
            name.to_ascii_lowercase(),
            Entry {
                archive,
                offset: u64::from(offset),
                size,
                packed,
            },
        ));
    }
    Ok(out)
}

/// PNG to RGBA rows top down; whether it has an alpha channel.
pub fn decode_png(bytes: &[u8]) -> Result<(RgbaImage, bool)> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let (width, height) = (info.width, info.height);
    let pixels = width as usize * height as usize;
    let (rgba, alpha) = match info.color_type {
        png::ColorType::Rgba => (buf[..pixels * 4].to_vec(), true),
        png::ColorType::Rgb => (expand(&buf, pixels, |p| [p[0], p[1], p[2], 255], 3), false),
        png::ColorType::GrayscaleAlpha => {
            (expand(&buf, pixels, |p| [p[0], p[0], p[0], p[1]], 2), true)
        }
        png::ColorType::Grayscale => (expand(&buf, pixels, |p| [p[0], p[0], p[0], 255], 1), false),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    Ok((RgbaImage::new(width, height, rgba)?, alpha))
}

fn expand(buf: &[u8], pixels: usize, f: impl Fn(&[u8]) -> [u8; 4], channels: usize) -> Vec<u8> {
    buf[..pixels * channels]
        .chunks_exact(channels)
        .flat_map(f)
        .collect()
}

fn extension(path: &Path) -> Option<String> {
    Some(path.extension()?.to_str()?.to_ascii_lowercase())
}

/// A directory entry of `dir` named `name` in any letter case.
fn child_ignoring_case(dir: &Path, name: &str) -> Result<Option<PathBuf>> {
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        if item
            .file_name()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            && item.file_type()?.is_dir()
        {
            return Ok(Some(item.path()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(dir: &Path, name: &str, files: &[(&str, &[u8])]) {
        let mut table = Vec::new();
        let mut data = Vec::new();
        let start = 12 + files.len() * ENTRY_SIZE;
        for (name, bytes) in files {
            let packed = miniz_oxide::deflate::compress_to_vec(bytes, 6);
            let mut entry = vec![0u8; ENTRY_SIZE];
            entry[0..4].copy_from_slice(&((start + data.len()) as u32).to_le_bytes());
            entry[4..8].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            entry[8..12].copy_from_slice(&(packed.len() as u32).to_le_bytes());
            for (i, u) in name.encode_utf16().enumerate() {
                entry[NAME_OFFSET + 2 * i..NAME_OFFSET + 2 * i + 2]
                    .copy_from_slice(&u.to_le_bytes());
            }
            table.extend(entry);
            data.extend(packed);
        }
        let mut out = b"PKP\0\0\0\0\0".to_vec();
        out.extend((files.len() as u32).to_le_bytes());
        out.extend(table);
        out.extend(data);
        std::fs::write(dir.join(name), out).unwrap();
    }

    #[test]
    fn reads_scripts_images_and_movies_from_the_pk_folder() {
        let root = std::env::temp_dir().join(format!("cmvs-pk-{}", std::process::id()));
        let pk = root.join("Pk");
        std::fs::create_dir_all(&pk).unwrap();
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[1, 2, 3, 4, 5, 6]).unwrap();
        }
        archive(
            &pk,
            "Scripts.pk",
            &[("Main.ps3", b"PS2A main"), ("start.ps3", b"PS2A start")],
        );
        archive(&pk, "Graphic.pk", &[("logo.png", &png_bytes)]);
        std::fs::write(pk.join("op.cmv"), b"").unwrap();

        let patch = PkPatch::find(&root).unwrap().unwrap();
        assert_eq!(patch.len(), 3);
        assert_eq!(patch.script(b"main.ps3").unwrap().unwrap(), b"PS2A main");
        assert!(patch.script(b"other.ps3").unwrap().is_none());
        let raw = patch.image(b"LOGO.pb3").unwrap().unwrap();
        let (image, alpha) = cmvs_core::patch::parse_raw_image(&raw).unwrap();
        assert!(!alpha);
        assert_eq!(image.pixels(), &[1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(patch.movie_directory(), Some(b"Pk\\".as_slice()));
        std::fs::remove_dir_all(&root).unwrap();
        assert!(PkPatch::find(&root.parent().unwrap().join("no-such-game")).is_err());
    }
}
