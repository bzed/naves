// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes fetched imagery as GeoTIFF: EPSG:4326, 8 bit RGB, DEFLATE, `NoData` 0.

use std::io::{Cursor, Write};
use std::path::Path;

use naves_core::landcover::GeoWindow;
use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
use tiff::tags::Tag;

use crate::imagery_fetch::FetchError;

/// An RGB picture: `width * height * 3` bytes, row 0 at the north. Pixels of `[0, 0, 0]` are no data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbMosaic {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The colours, red, green, blue of every pixel.
    pub data: Vec<u8>,
}

/// `GDAL_NODATA`, the tag that says which value is no data.
const GDAL_NODATA: u16 = 42113;

fn tiff_error(path: &Path, e: &tiff::TiffError) -> FetchError {
    FetchError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::other(e.to_string()),
    }
}

/// Writes `rgb`, which covers `bounds`, to `path` (below an existing or new folder) atomically: the data goes
/// to a temporary file in the same folder and replaces `path` only when it is whole.
///
/// # Errors
///
/// [`FetchError::Other`] for a mosaic of the wrong size or bounds that are not a box, [`FetchError::Io`] if the
/// file cannot be written.
pub fn write_rgb(path: &Path, rgb: &RgbMosaic, bounds: GeoWindow) -> Result<(), FetchError> {
    let expected = usize::try_from(rgb.width)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(rgb.height).ok()?))
        .and_then(|p| p.checked_mul(3));
    if rgb.width == 0 || rgb.height == 0 || expected != Some(rgb.data.len()) {
        return Err(FetchError::Other(format!(
            "a picture of {} x {} pixels needs width x height x 3 bytes, not {}",
            rgb.width,
            rgb.height,
            rgb.data.len()
        )));
    }
    let finite = [bounds.south, bounds.west, bounds.north, bounds.east]
        .iter()
        .all(|v| v.is_finite());
    if !finite || bounds.east <= bounds.west || bounds.north <= bounds.south {
        return Err(FetchError::Other(
            "the bounds of the picture are not a box".to_string(),
        ));
    }
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor)
            .map_err(|e| tiff_error(path, &e))?
            .with_compression(Compression::Deflate(DeflateLevel::default()));
        let mut image = encoder
            .new_image::<colortype::RGB8>(rgb.width, rgb.height)
            .map_err(|e| tiff_error(path, &e))?;
        let scale = [
            (bounds.east - bounds.west) / f64::from(rgb.width),
            (bounds.north - bounds.south) / f64::from(rgb.height),
            0.0,
        ];
        let tie = [0.0, 0.0, 0.0, bounds.west, bounds.north, 0.0];
        // GeoKeyDirectory: version 1.1.0, three keys: model type geographic, raster type area, EPSG:4326
        let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326];
        let tags = image.encoder();
        tags.write_tag(Tag::ModelPixelScaleTag, &scale[..])
            .and_then(|()| tags.write_tag(Tag::ModelTiepointTag, &tie[..]))
            .and_then(|()| tags.write_tag(Tag::GeoKeyDirectoryTag, &keys[..]))
            .and_then(|()| tags.write_tag(Tag::Unknown(GDAL_NODATA), "0"))
            .map_err(|e| tiff_error(path, &e))?;
        image
            .write_data(&rgb.data)
            .map_err(|e| tiff_error(path, &e))?;
    }
    let io = |source: std::io::Error| FetchError::Io {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(io)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".naves-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(io)?;
    tmp.write_all(cursor.get_ref())
        .and_then(|()| tmp.flush())
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(io)?;
    tmp.persist(path).map_err(|e| io(e.error))?;
    Ok(())
}
