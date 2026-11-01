//! Files of a translation patch that take precedence over the game's own.
//!
//! Fan translations of CMVS games ship a DLL that hooks the engine's file
//! reads and serves scripts and images from its own archives (for example
//! the `Pk/*.pk` of アマツツミ's Chinese patch). The host reads those
//! archives and hands the files to the runner through this trait.

use crate::{FormatError, raster::RgbaImage};
use std::error::Error;

pub trait PatchFiles {
    /// A PS2A script replacing `name` (as scripts name it, e.g. `main.ps3`).
    fn script(&self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
    /// An image replacing `name` (as scripts name it, e.g. `cg_bg.pb3`), in a
    /// format `image::decode` reads, such as [`raw_image`].
    fn image(&self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
    /// A game-relative directory (ending in `\`) searched for movies before
    /// the 0x2015 movie directory.
    fn movie_directory(&self) -> Option<&[u8]> {
        None
    }
}

/// Magic of decoded pixels a patch hands over in place of an encoded image.
pub const RAW_IMAGE_MAGIC: &[u8; 4] = b"RGBA"; // matched literally in image::decode

/// Wraps decoded pixels for `PatchFiles::image`: magic, width, height, an
/// alpha flag (the image has 32 bits per pixel), then RGBA rows top down.
pub fn raw_image(image: &RgbaImage, alpha: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + image.pixels().len());
    out.extend_from_slice(RAW_IMAGE_MAGIC);
    out.extend_from_slice(&image.width().to_le_bytes());
    out.extend_from_slice(&image.height().to_le_bytes());
    out.extend_from_slice(&u32::from(alpha).to_le_bytes());
    out.extend_from_slice(image.pixels());
    out
}

/// Inverse of [`raw_image`].
pub fn parse_raw_image(bytes: &[u8]) -> Result<(RgbaImage, bool), FormatError> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(FormatError("truncated raw patch image"))
    };
    if !bytes.starts_with(RAW_IMAGE_MAGIC) {
        return Err(FormatError("expected a raw patch image"));
    }
    let (width, height, alpha) = (word(4)?, word(8)?, word(12)?);
    Ok((
        RgbaImage::new(width, height, bytes[16..].to_vec())?,
        alpha != 0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_images_round_trip() {
        let image = RgbaImage::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let (back, alpha) = parse_raw_image(&raw_image(&image, true)).unwrap();
        assert!(alpha);
        assert_eq!(back, image);
        assert!(parse_raw_image(&raw_image(&image, false)[..20]).is_err());
    }
}
