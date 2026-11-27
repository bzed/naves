// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Element ids: an OSM id, or a user id written `u<n>`.
//!
//! In the edit files an OSM id is a plain integer and a user id is the string `u<n>` with `n >= 1`. Inside the
//! program a user id is `USER_ID_BASE + n`, so that both kinds can share one number space that cannot collide
//! with an OSM id (those are below `USER_ID_BASE`).

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The first number of the user ids inside the program. OSM ids are all below it.
pub const USER_ID_BASE: i64 = 1 << 40;

/// Which element an edit is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementId {
    /// An element of OpenStreetMap, by its OSM id (`1..USER_ID_BASE`).
    Osm(i64),
    /// An element the user drew, `n >= 1`.
    User(u64),
}

/// A text that is not an element id.
#[derive(Debug, thiserror::Error)]
#[error("not an element id: {0:?} (expected an OSM id as an integer, or u<n> with n >= 1)")]
pub struct ElementIdError(String);

impl ElementId {
    /// The id inside the program: the OSM id itself, or `USER_ID_BASE + n` for a user id.
    ///
    /// `None` for an id that cannot be used: an OSM id outside `1..USER_ID_BASE`, a user id of 0, or one whose
    /// number does not fit.
    #[must_use]
    pub fn raw(self) -> Option<i64> {
        match self {
            ElementId::Osm(n) => (1..USER_ID_BASE).contains(&n).then_some(n),
            ElementId::User(n) => {
                let n = i64::try_from(n).ok().filter(|&n| n >= 1)?;
                n.checked_add(USER_ID_BASE)
            }
        }
    }

    /// The inverse of [`ElementId::raw`]: the id for a number inside the program.
    ///
    /// `None` for `raw <= 0`, and for `raw == USER_ID_BASE` (a user id needs `n >= 1`).
    #[must_use]
    pub fn from_raw(raw: i64) -> Option<ElementId> {
        if raw <= 0 {
            None
        } else if raw < USER_ID_BASE {
            Some(ElementId::Osm(raw))
        } else {
            let n = raw.checked_sub(USER_ID_BASE).filter(|&n| n >= 1)?;
            Some(ElementId::User(u64::try_from(n).ok()?))
        }
    }

    /// Whether the user drew this element.
    #[must_use]
    pub fn is_user(self) -> bool {
        matches!(self, ElementId::User(_))
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ElementId::Osm(n) => write!(f, "{n}"),
            ElementId::User(n) => write!(f, "u{n}"),
        }
    }
}

/// `true` for a non-empty text of ASCII digits only (no sign, no spaces).
fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

impl FromStr for ElementId {
    type Err = ElementIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || ElementIdError(text.to_string());
        let id = if let Some(digits) = text.strip_prefix('u') {
            if !is_digits(digits) {
                return Err(bad());
            }
            ElementId::User(digits.parse().map_err(|_| bad())?)
        } else {
            if !is_digits(text) {
                return Err(bad());
            }
            ElementId::Osm(text.parse().map_err(|_| bad())?)
        };
        // Only ids that have a program number are accepted.
        if id.raw().is_some() {
            Ok(id)
        } else {
            Err(bad())
        }
    }
}

impl Serialize for ElementId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ElementId::Osm(n) => serializer.serialize_i64(*n),
            ElementId::User(_) => serializer.collect_str(self),
        }
    }
}

/// Reads an id that is an integer (OSM) or a string `u<n>` (user).
struct IdVisitor;

impl Visitor<'_> for IdVisitor {
    type Value = ElementId;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an OSM id as an integer, or a user id as a string u<n>")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<ElementId, E> {
        let id = ElementId::Osm(value);
        if id.raw().is_some() {
            Ok(id)
        } else {
            Err(E::custom(format!(
                "OSM id {value} is outside 1..{USER_ID_BASE}"
            )))
        }
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<ElementId, E> {
        let value = i64::try_from(value).map_err(E::custom)?;
        self.visit_i64(value)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<ElementId, E> {
        value.parse().map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for ElementId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(IdVisitor)
    }
}
