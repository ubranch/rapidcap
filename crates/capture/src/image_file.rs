use std::{
    fmt,
    path::{Path, PathBuf},
};

use image::{
    ExtendedColorType, ImageEncoder,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};

use crate::settings::write_atomic;

pub fn save_screenshot(
    rgba: &[u8],
    width: u32,
    height: u32,
    base_path: &Path,
    png_to_jpeg_threshold_bytes: usize,
    jpeg_quality: u8,
) -> Result<PathBuf, ImageFileError> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(ImageFileError::InvalidPixels);
    }

    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(ImageFileError::encode)?;

    let final_path = if encoded.len() > png_to_jpeg_threshold_bytes {
        encoded.clear();
        // `collect()` here used to start from nothing: `flat_map` reports no
        // upper size hint, so a full-screen capture grew this buffer by doubling
        // and recopied tens of megabytes on the way. The size is known exactly.
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for pixel in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        JpegEncoder::new_with_quality(&mut encoded, jpeg_quality)
            .encode(&rgb, width, height, ExtendedColorType::Rgb8)
            .map_err(ImageFileError::encode)?;
        with_suffix(base_path, "jpg")
    } else {
        with_suffix(base_path, "png")
    };

    write_atomic(&with_suffix(base_path, "part"), &final_path, &encoded)
        .map_err(ImageFileError::io)?;
    Ok(final_path)
}

/// Appends `.extension`, where `Path::with_extension` would replace one.
///
/// The stem carries the process name, and shipped Windows executables are not
/// all one bare word: `Microsoft.Photos.exe`, `Microsoft.CmdPal.UI.exe`,
/// `python3.13.exe`. `with_extension` cuts at the *last* dot, so
/// `Microsoft.Photos_ab12cd34ef` came out as `Microsoft.png` - the random
/// suffix gone, and every capture of that app silently overwriting the one
/// before it.
fn with_suffix(base: &Path, extension: &str) -> PathBuf {
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(extension);
    base.with_file_name(name)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageFileError {
    InvalidPixels,
    Encode(String),
    Io(String),
}

impl ImageFileError {
    fn encode(error: image::ImageError) -> Self {
        Self::Encode(error.to_string())
    }

    fn io(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl fmt::Display for ImageFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPixels => formatter.write_str("pixel buffer dimensions do not match"),
            Self::Encode(message) => write!(formatter, "image encoding failed: {message}"),
            Self::Io(message) => write!(formatter, "image file I/O failed: {message}"),
        }
    }
}

impl std::error::Error for ImageFileError {}
