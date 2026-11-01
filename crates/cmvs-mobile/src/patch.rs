//! Translation patches that hook the engine with a DLL and serve their own
//! files. Three kinds are read, and a game may have several:
//!
//! * A `patch` folder of loose files named like the game's own (Mikage's
//!   format for anything unpacked by hand), see [`LoosePatch`]. It wins over
//!   the others.
//! * PKP archives (茶组's アマツツミ Chinese patch, `amatsu.dll`, ships them as
//!   `Pk/*.pk`), see [`PkPatch`].
//! * `Longinus.dat` (the Longinus hook, used by アオイトリ's Chinese patch),
//!   see [`LonginusPatch`].
//!
//! The archives are found by their content, not their names: any file in the
//! game folder or one folder below it (except `data` and `savedata`) that
//! starts with `PKP\0` or `Longinus`. Both known archives carry GBK text. A `.pk` file is `PKP\0`, a zero word, the entry count, then 0xb0-byte
//! entries: data offset, unpacked size, packed size, 36 zero bytes and a
//! NUL-padded UTF-16 name. Data is raw DEFLATE. Scripts are PS2A without the
//! byte obfuscation; images are PNG files named like the PB3 they replace
//! (`cg_bg.png` for `cg_bg.pb3`). Subtitled movies sit next to the archives
//! as plain `.cmv` files. The patched scripts carry GBK text.
use cmvs_core::{patch::PatchFiles, raster::RgbaImage};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The translation patches of a game, if it has any, layered as one, and a
/// line for the log.
pub fn find(root: &Path) -> Result<Option<(std::sync::Arc<dyn PatchFiles>, String)>> {
    let mut layers: Vec<Box<dyn PatchFiles + Send + Sync>> = Vec::new();
    let mut lines = Vec::new();
    if let Some(patch) = LoosePatch::find(root)? {
        lines.push(format!(
            "{} files in {}",
            patch.len(),
            patch.dir.file_name().unwrap_or_default().to_string_lossy()
        ));
        layers.push(Box::new(patch));
    }
    let found = scan(root)?;
    if !found.pkp.is_empty() {
        let patch = PkPatch::from_archives(root, found.pkp)?;
        lines.push(format!("{} files in PKP archives", patch.len()));
        layers.push(Box::new(patch));
    }
    if let Some(path) = found.longinus {
        let patch = LonginusPatch::open(root, path)?;
        lines.push(format!(
            "{} files in {}",
            patch.len(),
            patch.path.file_name().unwrap_or_default().to_string_lossy()
        ));
        layers.push(Box::new(patch));
    }
    Ok(match layers.len() {
        0 => None,
        1 => {
            let only: std::sync::Arc<dyn PatchFiles + Send + Sync> =
                std::sync::Arc::from(layers.pop().expect("one"));
            Some((only, lines.join("")))
        }
        _ => Some((std::sync::Arc::new(Layered(layers)), lines.join(", "))),
    })
}

/// Patches tried in order; the first that has a file wins.
struct Layered(Vec<Box<dyn PatchFiles + Send + Sync>>);

impl PatchFiles for Layered {
    fn script(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        for layer in &self.0 {
            if let Some(bytes) = layer.script(name)? {
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }
    fn image(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        // A whole image of an earlier layer wins over pixels of a later one.
        for layer in &self.0 {
            if let Some(bytes) = layer.image(name)? {
                return Ok(Some(bytes));
            }
            if layer.pixels(name)?.is_some() {
                return Ok(None);
            }
        }
        Ok(None)
    }
    fn pixels(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        for layer in &self.0 {
            if layer.image(name)?.is_some() {
                return Ok(None);
            }
            if let Some(bytes) = layer.pixels(name)? {
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }
    fn movie_directory(&self) -> Option<&[u8]> {
        self.0.iter().find_map(|layer| layer.movie_directory())
    }
}

#[derive(Default)]
struct Found {
    pkp: Vec<PathBuf>,
    longinus: Option<PathBuf>,
}

/// Bounds the content scan: game folders hold a few dozen files at the top.
const MAX_SCANNED: usize = 4096;

/// Patch archives in `root` and its direct subfolders, by their magic.
fn scan(root: &Path) -> Result<Found> {
    let mut found = Found::default();
    let mut dirs = vec![root.to_owned()];
    for item in std::fs::read_dir(root)? {
        let item = item?;
        let skip = item.file_name().to_str().is_some_and(|n| {
            ["data", "savedata", "save", "patch"]
                .iter()
                .any(|d| n.eq_ignore_ascii_case(d))
        });
        if item.file_type()?.is_dir() && !skip {
            dirs.push(item.path());
        }
    }
    let mut scanned = 0;
    for dir in dirs {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|item| item.ok())
            .filter(|item| item.file_type().is_ok_and(|t| t.is_file()))
            .map(|item| item.path())
            .collect();
        files.sort();
        for path in files {
            scanned += 1;
            if scanned > MAX_SCANNED {
                return Ok(found);
            }
            let mut head = [0u8; 8];
            let Ok(mut file) = File::open(&path) else {
                continue;
            };
            if file.read_exact(&mut head).is_err() {
                continue;
            }
            if head.starts_with(b"PKP\0") {
                found.pkp.push(path);
            } else if &head == b"Longinus" && found.longinus.is_none() {
                found.longinus = Some(path);
            }
        }
    }
    Ok(found)
}

/// Loose files in `<root>/patch` (any letter case) and its subfolders, named
/// like the files they replace: scripts `*.ps3` (as in the game's archives,
/// or decoded), images as the game's own format (`*.pb3`) or as PNG named
/// after it (`cg_bg.png` for `cg_bg.pb3`), and movies `*.cmv`. Folders only
/// group files: a name means the same file wherever it sits. GARbro and
/// similar tools extract the game's and many patches' archives to such files.
pub struct LoosePatch {
    dir: PathBuf,
    /// Lower-case file names.
    files: HashMap<String, PathBuf>,
    movies: Option<Vec<u8>>,
}

/// Bounds the loose folder walk.
const MAX_LOOSE_FILES: usize = 65536;
const MAX_LOOSE_DEPTH: usize = 4;

impl LoosePatch {
    /// `<root>/patch`, or None when the game has none or it is empty.
    pub fn find(root: &Path) -> Result<Option<Self>> {
        let Some(dir) = child_ignoring_case(root, "patch", true)? else {
            return Ok(None);
        };
        let mut files = HashMap::new();
        let mut movies = None;
        let mut pending = vec![(dir.clone(), 0)];
        while let Some((current, depth)) = pending.pop() {
            let mut items: Vec<_> = std::fs::read_dir(&current)?.collect::<std::io::Result<_>>()?;
            items.sort_by_key(|item| item.file_name());
            for item in items {
                let path = item.path();
                if item.file_type()?.is_dir() {
                    if depth + 1 < MAX_LOOSE_DEPTH {
                        pending.push((path, depth + 1));
                    }
                    continue;
                }
                let Some(name) = item.file_name().to_str().map(str::to_ascii_lowercase) else {
                    continue;
                };
                if files.len() >= MAX_LOOSE_FILES {
                    return Err(format!("{}: too many files", dir.display()).into());
                }
                if movies.is_none() && name.ends_with(".cmv") {
                    movies = game_relative_dir(root, &current);
                }
                files.entry(name).or_insert(path);
            }
        }
        if files.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self { dir, files, movies }))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        match self.files.get(&name.to_ascii_lowercase()) {
            Some(path) => Ok(Some(std::fs::read(path)?)),
            None => Ok(None),
        }
    }
}

impl PatchFiles for LoosePatch {
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
        if !name.to_ascii_lowercase().ends_with(".png")
            && let Some(png) = self.read(&format!("{stem}.png"))?
        {
            let (image, alpha) =
                decode_png(&png).map_err(|e| format!("patch image {stem}.png: {e}"))?;
            return Ok(Some(cmvs_core::patch::raw_image(&image, alpha)));
        }
        self.read(name)
    }

    fn movie_directory(&self) -> Option<&[u8]> {
        self.movies.as_deref()
    }
}

/// `dir` relative to `root` with `\` separators and a trailing `\`, as the
/// engine names directories.
fn game_relative_dir(root: &Path, dir: &Path) -> Option<Vec<u8>> {
    let relative = dir.strip_prefix(root).ok()?;
    let mut out = Vec::new();
    for part in relative.components() {
        out.extend_from_slice(part.as_os_str().to_str()?.as_bytes());
        out.push(b'\\');
    }
    Some(out)
}

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
    movies: Option<Vec<u8>>,
}

impl PkPatch {
    /// The patch in `<root>/Pk`, or None when the game has none.
    pub fn find(root: &Path) -> Result<Option<Self>> {
        let Some(dir) = child_ignoring_case(root, "pk", true)? else {
            return Ok(None);
        };
        let mut archives = Vec::new();
        for item in std::fs::read_dir(&dir)? {
            let path = item?.path();
            if extension(&path).as_deref() == Some("pk") {
                archives.push(path);
            }
        }
        if archives.is_empty() {
            return Ok(None);
        }
        Self::from_archives(root, archives).map(Some)
    }

    /// PKP archives found anywhere in the game folder. Subtitled movies are
    /// looked for next to the first archive.
    pub fn from_archives(root: &Path, mut archives: Vec<PathBuf>) -> Result<Self> {
        archives.sort_by_key(|path| path.file_name().map(|n| n.to_owned()));
        let movies = archives
            .first()
            .and_then(|path| path.parent())
            .filter(|dir| {
                std::fs::read_dir(dir).is_ok_and(|items| {
                    items
                        .flatten()
                        .any(|item| extension(&item.path()).as_deref() == Some("cmv"))
                })
            })
            .and_then(|dir| game_relative_dir(root, dir));
        let mut entries = BTreeMap::new();
        for (archive, path) in archives.iter().enumerate() {
            for (name, entry) in
                read_index(path, archive).map_err(|e| format!("{}: {e}", path.display()))?
            {
                entries.insert(name, entry);
            }
        }
        Ok(Self {
            archives,
            entries,
            movies,
        })
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
        self.movies.as_deref()
    }
}

/// `Longinus.dat`: `Longinus`, a version word (01 04 00 01), the entry count,
/// then 12-byte entries (name hash, size, offset) and the files, stored as is.
/// The hash is BKDR with seed 131 over the UTF-16LE bytes of the upper-case
/// name, masked to 31 bits (Longinus.dll 0x1000540f). Scripts are PS2A as in
/// the game's archives. Images are the decoded pixels, BGRA rows top down at
/// the size of the PB3 they replace; Longinus.ini names the engine's file-read
/// and picture-copy sites it hooks for both.
///
/// The patch also rewrites the exe's `ps3` of `start.ps3` to `CHS`, so the
/// game starts from `data/pack/start.CHS` (the game's format, with the window
/// title translated). The `start.ps3` inside the archive is a leftover that
/// reads loose image folders the game does not have, and is not used.
pub struct LonginusPatch {
    path: PathBuf,
    entries: HashMap<u32, (u32, u32)>,
    start: Option<PathBuf>,
}

impl LonginusPatch {
    /// `<root>/Longinus.dat`, or None when the game has none.
    pub fn find(root: &Path) -> Result<Option<Self>> {
        let Some(path) = child_ignoring_case(root, "longinus.dat", false)? else {
            return Ok(None);
        };
        Self::open(root, path).map(Some)
    }

    /// A Longinus archive at `path`, under any name.
    pub fn open(root: &Path, path: PathBuf) -> Result<Self> {
        let mut file = File::open(&path)?;
        let length = file.metadata()?.len();
        let mut head = [0u8; 16];
        file.read_exact(&mut head)?;
        if &head[..8] != b"Longinus" {
            return Err(format!("{}: not a Longinus archive", path.display()).into());
        }
        let count = u32::from_le_bytes(head[12..16].try_into().expect("4 bytes"));
        if count > MAX_ENTRIES {
            return Err(format!("{}: too many entries", path.display()).into());
        }
        let mut table = vec![0u8; count as usize * 12];
        file.read_exact(&mut table)?;
        let mut entries = HashMap::new();
        for raw in table.chunks_exact(12) {
            let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().expect("4 bytes"));
            let (hash, size, offset) = (word(0), word(4), word(8));
            if size > MAX_FILE || u64::from(offset) + u64::from(size) > length {
                return Err(format!("{}: entry outside the archive", path.display()).into());
            }
            entries.insert(hash, (size, offset));
        }
        let start = match child_ignoring_case(root, "data", true)? {
            Some(data) => match child_ignoring_case(&data, "pack", true)? {
                Some(pack) => child_ignoring_case(&pack, "start.chs", false)?,
                None => None,
            },
            None => None,
        };
        Ok(Self {
            path,
            entries,
            start,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Longinus.dll 0x1000540f.
    pub fn hash(name: &str) -> u32 {
        let mut hash = 0u32;
        for unit in name.to_ascii_uppercase().encode_utf16() {
            for byte in unit.to_le_bytes() {
                hash = hash.wrapping_mul(131).wrapping_add(u32::from(byte));
            }
        }
        hash & 0x7fff_ffff
    }

    fn read(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        let Ok(name) = std::str::from_utf8(name) else {
            return Ok(None);
        };
        let Some(&(size, offset)) = self.entries.get(&Self::hash(name)) else {
            return Ok(None);
        };
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut data = vec![0; size as usize];
        file.read_exact(&mut data)?;
        Ok(Some(data))
    }
}

impl PatchFiles for LonginusPatch {
    fn script(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        if name.eq_ignore_ascii_case(b"start.ps3") {
            return match &self.start {
                Some(path) => Ok(Some(std::fs::read(path)?)),
                None => Ok(None),
            };
        }
        self.read(name)
    }
    fn image(&self, _name: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn pixels(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        self.read(name)
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
fn child_ignoring_case(dir: &Path, name: &str, directory: bool) -> Result<Option<PathBuf>> {
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        if item
            .file_name()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            && item.file_type()?.is_dir() == directory
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
    #[test]
    fn finds_renamed_archives_and_layers_the_loose_folder_first() {
        let root = std::env::temp_dir().join(format!("cmvs-loose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let movies = root.join("Patch").join("Movie");
        let other = root.join("中文补丁");
        std::fs::create_dir_all(&movies).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(root.join("data").join("pack")).unwrap();
        // A PKP archive under a name and folder of its own, found by content.
        archive(
            &other,
            "res.bin",
            &[("main.ps3", b"PS2A pk main"), ("sn1.ps3", b"PS2A pk sn1")],
        );
        std::fs::write(other.join("readme.txt"), b"hello").unwrap();
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[9, 8, 7, 6]).unwrap();
        }
        std::fs::write(root.join("Patch").join("MAIN.ps3"), b"PS2A loose main").unwrap();
        std::fs::write(root.join("Patch").join("title.png"), &png_bytes).unwrap();
        std::fs::write(root.join("Patch").join("logo.pb3"), b"PB3 bytes").unwrap();
        std::fs::write(movies.join("op.cmv"), b"").unwrap();

        let (patch, line) = find(&root).unwrap().unwrap();
        assert_eq!(line, "4 files in Patch, 2 files in PKP archives");
        assert_eq!(
            patch.script(b"main.ps3").unwrap().unwrap(),
            b"PS2A loose main"
        );
        assert_eq!(patch.script(b"sn1.ps3").unwrap().unwrap(), b"PS2A pk sn1");
        assert!(patch.script(b"sn2.ps3").unwrap().is_none());
        let (image, alpha) =
            cmvs_core::patch::parse_raw_image(&patch.image(b"title.pb3").unwrap().unwrap())
                .unwrap();
        assert!(alpha);
        assert_eq!(image.pixels(), &[9, 8, 7, 6]);
        assert_eq!(patch.image(b"LOGO.pb3").unwrap().unwrap(), b"PB3 bytes");
        assert_eq!(patch.movie_directory(), Some(b"Patch\\Movie\\".as_slice()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn whole_images_of_an_earlier_layer_hide_later_pixels() {
        struct Pixels;
        impl PatchFiles for Pixels {
            fn script(&self, _: &[u8]) -> Result<Option<Vec<u8>>> {
                Ok(None)
            }
            fn image(&self, _: &[u8]) -> Result<Option<Vec<u8>>> {
                Ok(None)
            }
            fn pixels(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
                Ok((name == b"a.pb3" || name == b"b.pb3").then(|| vec![1]))
            }
        }
        struct Whole;
        impl PatchFiles for Whole {
            fn script(&self, _: &[u8]) -> Result<Option<Vec<u8>>> {
                Ok(None)
            }
            fn image(&self, name: &[u8]) -> Result<Option<Vec<u8>>> {
                Ok((name == b"a.pb3").then(|| vec![2]))
            }
        }
        let layered = Layered(vec![Box::new(Whole), Box::new(Pixels)]);
        assert_eq!(layered.image(b"a.pb3").unwrap(), Some(vec![2]));
        assert_eq!(layered.pixels(b"a.pb3").unwrap(), None);
        assert_eq!(layered.image(b"b.pb3").unwrap(), None);
        assert_eq!(layered.pixels(b"b.pb3").unwrap(), Some(vec![1]));
    }

    #[test]
    fn longinus_hash_matches_the_hook_log() {
        // CMVS_hook.log of アオイトリ's patch: sn401.ps3|Hash:0x71C45A76.
        assert_eq!(LonginusPatch::hash("sn401.ps3"), 0x71c4_5a76);
        assert_eq!(LonginusPatch::hash("sn108.ps3"), 0x001d_6d32);
    }

    #[test]
    fn reads_scripts_pixels_and_the_chs_start_from_longinus() {
        let root = std::env::temp_dir().join(format!("cmvs-longinus-{}", std::process::id()));
        let pack = root.join("data").join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        let files: [(&str, &[u8]); 3] = [
            ("sn101.ps3", b"PS2A sn101"),
            ("logo.pb3", &[1, 2, 3, 4]),
            ("start.ps3", b"PS2A leftover"),
        ];
        let mut out = b"Longinus\x01\x04\x00\x01".to_vec();
        out.extend((files.len() as u32).to_le_bytes());
        let mut offset = 16 + 12 * files.len();
        for (name, bytes) in files {
            out.extend(LonginusPatch::hash(name).to_le_bytes());
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend((offset as u32).to_le_bytes());
            offset += bytes.len();
        }
        for (_, bytes) in files {
            out.extend(bytes);
        }
        std::fs::write(root.join("Longinus.dat"), out).unwrap();

        // Without start.CHS the game's own start.ps3 is used.
        let patch = LonginusPatch::find(&root).unwrap().unwrap();
        assert_eq!(patch.len(), 3);
        assert!(patch.script(b"start.ps3").unwrap().is_none());
        assert_eq!(patch.script(b"SN101.PS3").unwrap().unwrap(), b"PS2A sn101");
        assert_eq!(patch.pixels(b"logo.pb3").unwrap().unwrap(), [1, 2, 3, 4]);
        assert!(patch.image(b"logo.pb3").unwrap().is_none());
        assert!(patch.script(b"other.ps3").unwrap().is_none());

        std::fs::write(pack.join("start.CHS"), b"PS2A chs").unwrap();
        let (patch, line) = find(&root).unwrap().unwrap();
        assert_eq!(line, "3 files in Longinus.dat");
        assert_eq!(patch.script(b"start.ps3").unwrap().unwrap(), b"PS2A chs");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
