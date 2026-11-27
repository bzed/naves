// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes fetched imagery as GeoTIFF: EPSG:4326, 8 bit RGB, DEFLATE, `NoData` 0; and fetched heights as
//! one-band Float32 GeoTIFF in the same projection, DEFLATE, without a `NoData` tag.

use std::io::{Cursor, Write};
use std::path::Path;

use naves_core::landcover::GeoWindow;
use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, TiffValue, colortype};
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

/// A one-band float raster: `width * height` heights, row 0 at the north.
#[derive(Debug, Clone, PartialEq)]
pub struct DemMosaic {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The heights.
    pub data: Vec<f32>,
}

/// `GDAL_NODATA`, the tag that says which value is no data.
const GDAL_NODATA: u16 = 42113;

fn tiff_error(path: &Path, e: &tiff::TiffError) -> FetchError {
    FetchError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::other(e.to_string()),
    }
}

/// Whether `len` values make a `width` x `height` raster of `per_pixel` values per pixel; a zero side never does.
fn grid_fits(width: u32, height: u32, len: usize, per_pixel: usize) -> bool {
    usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(height).ok()?))
        .and_then(|p| p.checked_mul(per_pixel))
        == Some(len)
        && width != 0
        && height != 0
}

/// Refuses bounds that are not a box of finite edges.
fn check_box(bounds: GeoWindow) -> Result<(), FetchError> {
    let finite = [bounds.south, bounds.west, bounds.north, bounds.east]
        .iter()
        .all(|v| v.is_finite());
    if !finite || bounds.east <= bounds.west || bounds.north <= bounds.south {
        return Err(FetchError::Other(
            "the bounds of the picture are not a box".to_string(),
        ));
    }
    Ok(())
}

/// Encodes `data`, a `width` x `height` raster of `C` samples covering `bounds`, as an EPSG:4326 GeoTIFF in
/// memory. `no_data_zero` adds the `GDAL_NODATA` tag with the value 0.
fn encode<C>(
    path: &Path,
    width: u32,
    height: u32,
    bounds: GeoWindow,
    data: &[C::Inner],
    no_data_zero: bool,
) -> Result<Vec<u8>, FetchError>
where
    C: colortype::ColorType,
    [C::Inner]: TiffValue,
{
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor)
            .map_err(|e| tiff_error(path, &e))?
            .with_compression(Compression::Deflate(DeflateLevel::default()));
        let mut image = encoder
            .new_image::<C>(width, height)
            .map_err(|e| tiff_error(path, &e))?;
        let scale = [
            (bounds.east - bounds.west) / f64::from(width),
            (bounds.north - bounds.south) / f64::from(height),
            0.0,
        ];
        let tie = [0.0, 0.0, 0.0, bounds.west, bounds.north, 0.0];
        // GeoKeyDirectory: version 1.1.0, three keys: model type geographic, raster type area, EPSG:4326
        let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326];
        let tags = image.encoder();
        tags.write_tag(Tag::ModelPixelScaleTag, &scale[..])
            .and_then(|()| tags.write_tag(Tag::ModelTiepointTag, &tie[..]))
            .and_then(|()| tags.write_tag(Tag::GeoKeyDirectoryTag, &keys[..]))
            .map_err(|e| tiff_error(path, &e))?;
        if no_data_zero {
            tags.write_tag(Tag::Unknown(GDAL_NODATA), "0")
                .map_err(|e| tiff_error(path, &e))?;
        }
        image.write_data(data).map_err(|e| tiff_error(path, &e))?;
    }
    Ok(cursor.into_inner())
}

/// Writes `bytes` to `path` (below an existing or new folder) atomically: they go to a temporary file in the
/// same folder, which replaces `path` only when it is whole.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FetchError> {
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
    // a temporary file is private; the imagery of a project is as readable as the project's other files
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644))
            .map_err(io)?;
    }
    tmp.write_all(bytes)
        .and_then(|()| tmp.flush())
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(io)?;
    tmp.persist(path).map_err(|e| io(e.error))?;
    Ok(())
}

/// Writes `rgb`, which covers `bounds`, to `path` (below an existing or new folder) atomically: the data goes
/// to a temporary file in the same folder and replaces `path` only when it is whole.
///
/// # Errors
///
/// [`FetchError::Other`] for a mosaic of the wrong size or bounds that are not a box, [`FetchError::Io`] if the
/// file cannot be written.
pub fn write_rgb(path: &Path, rgb: &RgbMosaic, bounds: GeoWindow) -> Result<(), FetchError> {
    if !grid_fits(rgb.width, rgb.height, rgb.data.len(), 3) {
        return Err(FetchError::Other(format!(
            "a picture of {} x {} pixels needs width x height x 3 bytes, not {}",
            rgb.width,
            rgb.height,
            rgb.data.len()
        )));
    }
    check_box(bounds)?;
    let bytes = encode::<colortype::RGB8>(path, rgb.width, rgb.height, bounds, &rgb.data, true)?;
    write_atomic(path, &bytes)
}

/// Writes `dem`, which covers `bounds`, to `path` as one-band Float32 GeoTIFF (EPSG:4326, DEFLATE, no `NoData`
/// tag), atomically like [`write_rgb`]. Heights that are not finite (NaN for no data) are written as they are.
///
/// # Errors
///
/// [`FetchError::Other`] for a mosaic of the wrong size or bounds that are not a box, [`FetchError::Io`] if the
/// file cannot be written.
pub fn write_dem(path: &Path, dem: &DemMosaic, bounds: GeoWindow) -> Result<(), FetchError> {
    if !grid_fits(dem.width, dem.height, dem.data.len(), 1) {
        return Err(FetchError::Other(format!(
            "a DEM of {} x {} pixels needs width x height heights, not {}",
            dem.width,
            dem.height,
            dem.data.len()
        )));
    }
    check_box(bounds)?;
    let bytes =
        encode::<colortype::Gray32Float>(path, dem.width, dem.height, bounds, &dem.data, false)?;
    write_atomic(path, &bytes)
}
