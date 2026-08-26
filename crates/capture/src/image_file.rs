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
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect();
        JpegEncoder::new_with_quality(&mut encoded, jpeg_quality)
            .encode(&rgb, width, height, ExtendedColorType::Rgb8)
            .map_err(ImageFileError::encode)?;
        base_path.with_extension("jpg")
    } else {
        base_path.with_extension("png")
    };

    write_atomic(&base_path.with_extension("part"), &final_path, &encoded)
        .map_err(ImageFileError::io)?;
    Ok(final_path)
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
