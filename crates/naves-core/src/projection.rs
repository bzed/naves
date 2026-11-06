// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Conversion between WGS84 latitude/longitude and the UTM zone of the project centre.

use proj4rs::Proj;

/// Errors from the coordinate conversion.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// proj4rs rejected a definition or a coordinate.
    #[error("projection error: {0}")]
    Proj(String),
    /// The position is outside the range UTM can represent.
    #[error("position {lat},{lon} is outside the range of UTM")]
    OutOfRange {
        /// Latitude in degrees.
        lat: f64,
        /// Longitude in degrees.
        lon: f64,
    },
}

impl From<proj4rs::errors::Error> for ProjectionError {
    fn from(e: proj4rs::errors::Error) -> Self {
        Self::Proj(e.to_string())
    }
}

/// UTM on WGS84, in the zone that contains the area centre.
#[derive(Debug)]
pub struct LocalProjection {
    geo: Proj,
    utm: Proj,
}

impl LocalProjection {
    /// The UTM zone (with the hemisphere) of the position (`lat`, `lon`) in degrees.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::OutOfRange`] for non-finite input or latitudes beyond UTM's 84°N/80°S,
    /// [`ProjectionError::Proj`] if proj4rs rejects the definition.
    pub fn for_center(lat: f64, lon: f64) -> Result<Self, ProjectionError> {
        if !(lat.is_finite() && lon.is_finite() && (-80.0..=84.0).contains(&lat)) {
            return Err(ProjectionError::OutOfRange { lat, lon });
        }
        let zone = utm_zone(lon);
        let south = if lat < 0.0 { " +south" } else { "" };
        Ok(Self {
            geo: Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")?,
            utm: Proj::from_proj_string(&format!(
                "+proj=utm +zone={zone}{south} +datum=WGS84 +units=m +no_defs"
            ))?,
        })
    }

    /// (easting, northing) in metres of the position (`lat`, `lon`) in degrees.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::Proj`] if the position cannot be converted.
    pub fn to_utm(&self, lat: f64, lon: f64) -> Result<(f64, f64), ProjectionError> {
        let mut point = (lon.to_radians(), lat.to_radians());
        proj4rs::transform::transform(&self.geo, &self.utm, &mut point)?;
        finite(point, lat, lon)
    }

    /// (latitude, longitude) in degrees of the UTM position (`easting`, `northing`) in metres.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::Proj`] if the position cannot be converted.
    pub fn to_geo(&self, easting: f64, northing: f64) -> Result<(f64, f64), ProjectionError> {
        let mut point = (easting, northing);
        proj4rs::transform::transform(&self.utm, &self.geo, &mut point)?;
        let (lon, lat) = finite(point, northing, easting)?;
        Ok((lat.to_degrees(), lon.to_degrees()))
    }
}

fn finite(p: (f64, f64), lat: f64, lon: f64) -> Result<(f64, f64), ProjectionError> {
    if p.0.is_finite() && p.1.is_finite() {
        Ok(p)
    } else {
        Err(ProjectionError::OutOfRange { lat, lon })
    }
}

/// UTM zone number 1..=60 of a longitude in degrees.
fn utm_zone(lon: f64) -> u8 {
    let lon = lon.clamp(-180.0, 179.999_999);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "(lon + 180) / 6 is in 0.0..60.0 after the clamp"
    )]
    let index = ((lon + 180.0) / 6.0).floor() as u8;
    index + 1
}

/// The EPSG code of the UTM zone (WGS84) that contains the position: 326xx north of the equator, 327xx south.
#[must_use]
pub fn utm_epsg(lat: f64, lon: f64) -> u32 {
    let base = if lat < 0.0 { 32_700 } else { 32_600 };
    base + u32::from(utm_zone(lon))
}
