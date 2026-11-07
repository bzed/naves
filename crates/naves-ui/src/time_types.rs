//! Minimal time types ported from `re_log_types` 0.30.2 / `re_types_core` 0.30.2
//! (<https://github.com/rerun-io/rerun>, MIT OR Apache-2.0).
//!
//! Only the API surface actually used by `re_ui` is kept. All arrow/serialization
//! support and the `STATIC` sentinel semantics of the original `TimeInt` are removed:
//! here [`TimeInt`] is a plain `i64` newtype.

use std::ops::RangeInclusive;
use std::str::FromStr as _;

// ----------------------------------------------------------------------------
// TimeInt

/// A 64-bit number describing either nanoseconds or sequence numbers.
///
/// Must be matched with a [`TimeType`] to know what.
///
/// Used both for time points and durations.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeInt(pub i64);

impl TimeInt {
    /// Value used to represent the minimal temporal value a [`TimeInt`] can hold.
    ///
    /// This is _not_ `i64::MIN`, matching the semantics of the original rerun types.
    pub const MIN: Self = Self(i64::MIN + 1);

    /// Value used to represent the maximum temporal value a [`TimeInt`] can hold.
    pub const MAX: Self = Self(i64::MAX);

    pub const ZERO: Self = Self(0);

    /// Creates a new temporal [`TimeInt`].
    ///
    /// If `time` is `i64::MIN`, this will return [`TimeInt::MIN`].
    #[inline]
    pub fn new_temporal(time: i64) -> Self {
        Self(time.max(Self::MIN.0))
    }

    /// For time timelines.
    #[inline]
    pub fn from_secs(seconds: f64) -> Self {
        Self::new_temporal((seconds * 1e9).round() as _)
    }

    /// Clamp to valid non-static range.
    #[inline]
    pub fn saturated_temporal_i64(value: impl Into<i64>) -> Self {
        Self::new_temporal(value.into())
    }

    #[inline]
    pub const fn as_i64(self) -> i64 {
        self.0
    }

    #[inline]
    pub const fn as_f64(self) -> f64 {
        self.0 as _
    }
}

impl From<i64> for TimeInt {
    #[inline]
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<TimeInt> for i64 {
    #[inline]
    fn from(value: TimeInt) -> Self {
        value.0
    }
}

impl std::ops::Neg for TimeInt {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self(self.0.saturating_neg())
    }
}

impl std::ops::Add for TimeInt {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl std::ops::Sub for TimeInt {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_sub(rhs.0))
    }
}

// ----------------------------------------------------------------------------
// TimeRangeBoundary

/// Left or right boundary of a time range.
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum TimeRangeBoundary {
    /// Boundary is a value relative to the time cursor.
    CursorRelative(TimeInt),

    /// Boundary is an absolute value.
    Absolute(TimeInt),

    /// The boundary extends to infinity.
    Infinite,
}

impl TimeRangeBoundary {
    /// Put the boundary at the current time cursor.
    pub const AT_CURSOR: Self = Self::CursorRelative(TimeInt(0));

    /// Returns the time assuming this boundary is a start boundary.
    pub fn start_boundary_time(&self, cursor: TimeInt) -> TimeInt {
        match *self {
            Self::Absolute(time) => time,
            Self::CursorRelative(time) => cursor + time,
            Self::Infinite => TimeInt::MIN,
        }
    }

    /// Returns the correct time assuming this boundary is an end boundary.
    pub fn end_boundary_time(&self, cursor: TimeInt) -> TimeInt {
        match *self {
            Self::Absolute(time) => time,
            Self::CursorRelative(time) => cursor + time,
            Self::Infinite => TimeInt::MAX,
        }
    }
}

// ----------------------------------------------------------------------------
// TimeRange

/// Time range bounds for a specific timeline.
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub struct TimeRange {
    /// Low time boundary.
    pub start: TimeRangeBoundary,

    /// High time boundary.
    pub end: TimeRangeBoundary,
}

impl TimeRange {
    /// The range encompassing all time, from beginning to end.
    pub const EVERYTHING: Self = Self {
        // This means the beginning.
        start: TimeRangeBoundary::Infinite,

        // This means the end.
        end: TimeRangeBoundary::Infinite,
    };

    /// A range of zero length exactly at the time cursor.
    ///
    /// This is *not* the same as latest-at queries and queries the state that was logged exactly at the cursor.
    /// In contrast, latest-at queries each component's latest known state.
    pub const AT_CURSOR: Self = Self {
        start: TimeRangeBoundary::AT_CURSOR,
        end: TimeRangeBoundary::AT_CURSOR,
    };
}

// ----------------------------------------------------------------------------
// AbsoluteTimeRange

/// An absolute time range using [`TimeInt`].
///
/// Can be resolved from [`TimeRange`] (which *may* have relative bounds) using a given cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbsoluteTimeRange {
    pub min: TimeInt,
    pub max: TimeInt,
}

impl AbsoluteTimeRange {
    /// Contains no time at all.
    pub const EMPTY: Self = Self {
        min: TimeInt::MAX,
        max: TimeInt::MIN,
    };

    /// Contains all time.
    pub const EVERYTHING: Self = Self {
        min: TimeInt::MIN,
        max: TimeInt::MAX,
    };

    /// Creates a new temporal [`AbsoluteTimeRange`].
    #[inline]
    pub fn new(min: impl Into<TimeInt>, max: impl Into<TimeInt>) -> Self {
        let min = min.into();
        let max = max.into();
        Self { min, max }
    }

    #[inline]
    pub fn min(&self) -> TimeInt {
        self.min
    }

    #[inline]
    pub fn max(&self) -> TimeInt {
        self.max
    }

    #[inline]
    pub fn contains(&self, time: TimeInt) -> bool {
        self.min <= time && time <= self.max
    }

    pub fn from_relative_time_range(range: &TimeRange, cursor: impl Into<TimeInt>) -> Self {
        let cursor = cursor.into();

        let mut min = range.start.start_boundary_time(cursor);
        let mut max = range.end.end_boundary_time(cursor);

        if min > max {
            std::mem::swap(&mut min, &mut max);
        }

        Self::new(min, max)
    }
}

impl From<AbsoluteTimeRange> for RangeInclusive<TimeInt> {
    fn from(range: AbsoluteTimeRange) -> Self {
        range.min..=range.max
    }
}

// ----------------------------------------------------------------------------
// TimeType

/// The type of a [`TimeInt`] or timeline.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimeType {
    /// Used e.g. for frames in a film.
    Sequence,

    /// Duration measured in nanoseconds.
    DurationNs,

    /// Nanoseconds since unix epoch (1970-01-01 00:00:00 UTC).
    TimestampNs,
}

impl std::fmt::Display for TimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequence => f.write_str("sequence"),
            Self::DurationNs => f.write_str("duration"),
            Self::TimestampNs => f.write_str("timestamp"),
        }
    }
}

impl TimeType {
    pub fn format_sequence(time_int: TimeInt) -> String {
        Self::Sequence.format(time_int, TimestampFormat::utc())
    }

    pub fn format(
        &self,
        time_int: impl Into<TimeInt>,
        timestamp_format: TimestampFormat,
    ) -> String {
        let subsecond_decimals = 0..=6; // NOTE: we currently ignore sub-microsecond
        self.format_opt(time_int, timestamp_format, subsecond_decimals)
    }

    /// The format will omit trailing sub-second zeroes as far as `subsecond_decimals` permits it.
    pub fn format_opt(
        &self,
        time_int: impl Into<TimeInt>,
        timestamp_format: TimestampFormat,
        subsecond_decimals: RangeInclusive<usize>,
    ) -> String {
        let time_int = time_int.into();
        if time_int == TimeInt::MIN {
            "beginning".into()
        } else if time_int == TimeInt::MAX {
            "end".into()
        } else {
            match self {
                Self::Sequence => format!("#{}", crate::format::format_int(time_int.as_i64())),
                Self::DurationNs => Duration::from(time_int).format_secs(subsecond_decimals),
                Self::TimestampNs => {
                    Timestamp::from(time_int).format_opt(timestamp_format, subsecond_decimals)
                }
            }
        }
    }

    #[inline]
    pub fn format_utc(&self, time_int: TimeInt) -> String {
        self.format(time_int, TimestampFormat::utc())
    }

    #[inline]
    pub fn format_range(
        &self,
        time_range: AbsoluteTimeRange,
        timestamp_format: TimestampFormat,
    ) -> String {
        format!(
            "{}..={}",
            self.format(time_range.min(), timestamp_format),
            self.format(time_range.max(), timestamp_format)
        )
    }

    #[inline]
    pub fn format_range_utc(&self, time_range: AbsoluteTimeRange) -> String {
        self.format_range(time_range, TimestampFormat::utc())
    }
}

// ----------------------------------------------------------------------------
// Duration

/// A signed duration represented as nanoseconds.
#[derive(Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct Duration(i64);

impl Duration {
    pub const MAX: Self = Self(i64::MAX);
    const NANOS_PER_SEC: i64 = 1_000_000_000;

    #[inline]
    pub const fn from_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    #[inline]
    pub fn from_secs(secs: impl Into<f64>) -> Self {
        let secs = secs.into();
        Self::from_nanos((secs * Self::NANOS_PER_SEC as f64).round() as _)
    }

    #[inline]
    pub fn as_nanos(&self) -> i64 {
        self.0
    }

    #[inline]
    pub fn as_secs_f64(&self) -> f64 {
        self.0 as f64 * 1e-9
    }

    /// The format will omit trailing sub-second zeroes as far as `subsecond_decimals` permits it.
    pub fn format_secs(self, subsecond_decimals: RangeInclusive<usize>) -> String {
        crate::format::DurationFormatOptions::default()
            .with_always_sign(true)
            .with_only_seconds(true)
            .with_min_decimals(*subsecond_decimals.start())
            .with_max_decimals(*subsecond_decimals.end())
            .format_nanos(self.as_nanos())
    }

    pub fn exact_format(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &crate::format::DurationFormatOptions::default()
                .with_always_sign(true)
                .with_only_seconds(false)
                .with_min_decimals(0)
                .with_max_decimals(9)
                .format_nanos(self.as_nanos()),
        )
    }
}

impl From<std::time::Duration> for Duration {
    #[inline]
    fn from(duration: std::time::Duration) -> Self {
        Self::from_nanos(duration.as_nanos() as _)
    }
}

impl std::ops::Neg for Duration {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        // Handle negation without overflow:
        if self.0 == i64::MIN {
            Self(i64::MAX)
        } else {
            Self(-self.0)
        }
    }
}

impl From<Duration> for TimeInt {
    #[inline]
    fn from(duration: Duration) -> Self {
        Self::saturated_temporal_i64(duration.as_nanos())
    }
}

impl From<TimeInt> for Duration {
    #[inline]
    fn from(int: TimeInt) -> Self {
        Self::from_nanos(int.as_i64())
    }
}

impl std::str::FromStr for Duration {
    type Err = jiff::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = &crate::format::remove_number_formatting(s);
        let jiff_duration = jiff::SignedDuration::from_str(s)?;
        Ok(Self(jiff_duration.as_nanos() as i64))
    }
}

impl std::fmt::Debug for Duration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.exact_format(f)
    }
}

impl std::fmt::Display for Duration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.exact_format(f)
    }
}

// ----------------------------------------------------------------------------
// TimestampFormat

/// How to display a [`Timestamp`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TimestampFormatKind {
    /// Convert to the local timezone and display as such explicitly (e.g. with "+01" for CET).
    LocalTimezone,

    /// Convert to the local timezone and display as such without specifying the timezone.
    ///
    /// Note that in this case the representation is ambiguous.
    LocalTimezoneImplicit,

    /// Display as UTC.
    #[default]
    Utc,

    /// Show as seconds since unix epoch
    SecondsSinceUnixEpoch,
}

/// Controls whether the date part of a timestamp is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DateVisibility {
    /// Always show the date.
    ShowDate,

    /// Hide the date when it's today.
    #[default]
    HideDateToday,

    /// Always hide the date.
    HideDate,
}

/// How to display a [`Timestamp`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TimestampFormat {
    /// What kind of format to use.
    format_kind: TimestampFormatKind,

    /// For date-time format kinds: when should we show the date part?
    date_visibility: DateVisibility,

    /// For date-time format kinds, should we omit date, nanos and suffix?
    short: bool,
}

impl From<TimestampFormatKind> for TimestampFormat {
    fn from(value: TimestampFormatKind) -> Self {
        Self {
            format_kind: value,
            ..Default::default()
        }
    }
}

impl TimestampFormat {
    pub fn utc() -> Self {
        Self::from(TimestampFormatKind::Utc)
    }

    pub fn local_timezone() -> Self {
        Self::from(TimestampFormatKind::LocalTimezone)
    }

    pub fn local_timezone_implicit() -> Self {
        Self::from(TimestampFormatKind::LocalTimezoneImplicit)
    }

    pub fn unix_epoch() -> Self {
        Self::from(TimestampFormatKind::SecondsSinceUnixEpoch)
    }

    pub fn kind(&self) -> TimestampFormatKind {
        self.format_kind
    }

    pub fn with_date_visibility(mut self, date_visibility: DateVisibility) -> Self {
        self.date_visibility = date_visibility;
        self
    }

    pub fn with_short(mut self, short: bool) -> Self {
        self.short = short;
        self
    }

    pub fn date_visibility(&self) -> DateVisibility {
        self.date_visibility
    }

    pub fn short(&self) -> bool {
        self.short
    }

    pub fn to_jiff_time_zone(self) -> jiff::tz::TimeZone {
        use jiff::tz::TimeZone;

        match self.format_kind {
            TimestampFormatKind::SecondsSinceUnixEpoch | TimestampFormatKind::Utc => TimeZone::UTC,

            TimestampFormatKind::LocalTimezone | TimestampFormatKind::LocalTimezoneImplicit => {
                TimeZone::try_system().unwrap_or_else(|err| {
                    log::warn!("Failed to detect system/local time zone: {err}");
                    TimeZone::UTC
                })
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Timestamp

/// Encodes a timestamp in nanoseconds since unix epoch.
///
/// Can represent any time between the years 1678 - 2261 CE to nanosecond precision.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    #[inline]
    pub fn now() -> Self {
        let nanos_since_epoch = web_time::SystemTime::UNIX_EPOCH
            .elapsed()
            .expect("Expected system clock to be set to after 1970")
            .as_nanos() as _;
        Self(nanos_since_epoch)
    }

    #[inline]
    pub fn from_nanos_since_epoch(nanos_since_epoch: i64) -> Self {
        Self(nanos_since_epoch)
    }

    #[inline]
    pub fn from_secs_since_epoch(secs: f64) -> Self {
        Self::from_nanos_since_epoch((secs * 1e9).round() as _)
    }

    #[inline]
    pub fn nanos_since_epoch(self) -> i64 {
        self.0
    }

    #[inline]
    pub fn elapsed(self) -> Duration {
        Self::now() - self
    }
}

// ------------------------------------------
// System converters

impl From<TimeInt> for Timestamp {
    #[inline]
    fn from(int: TimeInt) -> Self {
        Self::from_nanos_since_epoch(int.as_i64())
    }
}

impl From<Timestamp> for TimeInt {
    #[inline]
    fn from(timestamp: Timestamp) -> Self {
        Self::saturated_temporal_i64(timestamp.nanos_since_epoch())
    }
}

impl TryFrom<std::time::SystemTime> for Timestamp {
    type Error = std::time::SystemTimeError;

    fn try_from(time: std::time::SystemTime) -> Result<Self, Self::Error> {
        time.duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|duration_since_epoch| Self(duration_since_epoch.as_nanos() as _))
    }
}

// ------------------------------------------
// `jiff` converters

impl Timestamp {
    pub fn to_jiff_zoned(self, timestamp_format: TimestampFormat) -> jiff::Zoned {
        jiff::Timestamp::from(self).to_zoned(timestamp_format.to_jiff_time_zone())
    }
}

#[allow(clippy::fallible_impl_from)]
impl From<Timestamp> for jiff::Timestamp {
    fn from(value: Timestamp) -> Self {
        // Cannot fail - see docs for jiff::Timestamp::from_nanosecond
        #[allow(clippy::unwrap_used)]
        Self::from_nanosecond(value.nanos_since_epoch() as i128).unwrap()
    }
}

impl From<jiff::Timestamp> for Timestamp {
    fn from(value: jiff::Timestamp) -> Self {
        Self(value.as_nanosecond() as i64)
    }
}

impl From<jiff::Zoned> for Timestamp {
    fn from(value: jiff::Zoned) -> Self {
        value.timestamp().into()
    }
}

// ------------------------------------------
// Formatting and parsing

impl std::str::FromStr for Timestamp {
    type Err = jiff::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = &crate::format::remove_number_formatting(s);
        let jiff_timestamp = jiff::Timestamp::from_str(s)?;
        Ok(Self(jiff_timestamp.as_nanosecond() as i64))
    }
}

impl std::fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.format_iso().fmt(f)
    }
}

impl Timestamp {
    /// Formats the time as specified by ISO standard [`RFC3339`](https://www.rfc-editor.org/rfc/rfc3339.html).
    pub fn format_iso(self) -> String {
        jiff::Timestamp::from(self).to_string()
    }

    /// Human-readable timestamp.
    ///
    /// Omits the date of same-day timestamps.
    pub fn format(self, timestamp_format: TimestampFormat) -> String {
        let subsecond_decimals = 0..=6; // NOTE: we currently ignore sub-microsecond
        self.format_opt(timestamp_format, subsecond_decimals)
    }

    /// Human-readable timestamp.
    ///
    /// Omits the date of same-day timestamps.
    ///
    /// The format will omit trailing sub-second zeroes as far as `subsecond_decimals` permits it.
    pub fn format_opt(
        self,
        timestamp_format: TimestampFormat,
        subsecond_decimals: RangeInclusive<usize>,
    ) -> String {
        let format_fractional_nanos = move |ns: i32| {
            crate::format::DurationFormatOptions::default()
                .with_always_sign(false)
                .with_min_decimals(*subsecond_decimals.start())
                .with_max_decimals(*subsecond_decimals.end())
                .round_towards_zero()
                .format_nanos(ns as _)
                // Turn `0.123s` into `.123`:
                .trim_start_matches('0')
                .trim_end_matches('s')
                .to_owned()
        };

        let timestamp = jiff::Timestamp::from(self);

        match timestamp_format.kind() {
            TimestampFormatKind::SecondsSinceUnixEpoch => {
                format!(
                    "{}{}",
                    crate::format::format_int(timestamp.as_second()),
                    format_fractional_nanos(timestamp.subsec_nanosecond())
                )
            }

            TimestampFormatKind::LocalTimezone
            | TimestampFormatKind::LocalTimezoneImplicit
            | TimestampFormatKind::Utc => {
                let tz = timestamp_format.to_jiff_time_zone();
                let zoned = timestamp.to_zoned(tz.clone());

                let hide_date = match timestamp_format.date_visibility() {
                    // `ShowDate` takes priority over `short`, so a caller can get a date with
                    // whole seconds, no nanos and no suffix.
                    DateVisibility::ShowDate => false,
                    DateVisibility::HideDate => true,
                    DateVisibility::HideDateToday => {
                        timestamp_format.short()
                            || zoned.date() == jiff::Timestamp::now().to_zoned(tz.clone()).date()
                    }
                };
                let formatted = if hide_date {
                    zoned.strftime("%H:%M:%S").to_string()
                } else {
                    zoned.strftime("%Y-%m-%d %H:%M:%S").to_string()
                };

                let nanos = if timestamp_format.short() {
                    String::new()
                } else {
                    format_fractional_nanos(zoned.subsec_nanosecond())
                };

                let suffix = if timestamp_format.short() {
                    String::new()
                } else {
                    match timestamp_format.kind() {
                        TimestampFormatKind::LocalTimezone => tz.to_offset(timestamp).to_string(),
                        TimestampFormatKind::LocalTimezoneImplicit => String::new(),
                        TimestampFormatKind::Utc | TimestampFormatKind::SecondsSinceUnixEpoch => {
                            "Z".to_owned()
                        }
                    }
                };

                format!("{formatted}{nanos}{suffix}",)
            }
        }
    }

    /// Useful when showing dates/times on a timeline and you want it compact.
    ///
    /// Shows dates when zoomed out, shows times when zoomed in,
    /// shows relative millisecond when really zoomed in.
    pub fn format_time_compact(self, timestamp_format: TimestampFormat) -> String {
        match timestamp_format.kind() {
            TimestampFormatKind::SecondsSinceUnixEpoch => {
                let ns = self.nanos_since_epoch();
                let fractional_nanos = ns % 1_000_000_000;
                let is_whole_second = fractional_nanos == 0;
                if is_whole_second {
                    crate::format::format_int(ns / 1_000_000_000)
                } else {
                    // Show offset since last whole second:
                    Duration::from_nanos(fractional_nanos).format_subsecond_as_relative()
                }
            }

            TimestampFormatKind::LocalTimezone
            | TimestampFormatKind::LocalTimezoneImplicit
            | TimestampFormatKind::Utc => {
                let zoned = self.to_jiff_zoned(timestamp_format);
                if zoned.time() == jiff::civil::Time::MIN {
                    // Exactly midnight - show only the date:
                    zoned.strftime("%Y-%m-%d").to_string()
                } else if zoned.subsec_nanosecond() != 0 {
                    // Show offset since last whole second:
                    Duration::from_nanos(zoned.subsec_nanosecond() as _)
                        .format_subsecond_as_relative()
                } else if zoned.second() == 0 {
                    zoned.strftime("%H:%M").to_string()
                } else {
                    zoned.strftime("%H:%M:%S").to_string()
                }
            }
        }
    }

    /// Parse a timestamp.
    ///
    /// If it is missing a timezone specifier, the given timezone is assumed.
    pub fn parse_with_format(s: &str, timestamp_format: TimestampFormat) -> Option<Self> {
        let s = &crate::format::remove_number_formatting(s);

        if let Ok(utc) = Self::from_str(s) {
            // It has a `Z` suffix
            Some(utc)
        } else if let Ok(zoned) = jiff::Zoned::from_str(s) {
            // It had a timezone suffix
            Some(Self::from(zoned))
        } else if let Ok(date_time) = jiff::civil::DateTime::from_str(s) {
            date_time
                .to_zoned(timestamp_format.to_jiff_time_zone())
                .ok()
                .map(|zoned| zoned.into())
        } else if timestamp_format.kind() == TimestampFormatKind::SecondsSinceUnixEpoch {
            // Parse as seconds and convert to nanoseconds
            let seconds = s.parse::<f64>().ok()?;
            Some(Self::from_secs_since_epoch(seconds))
        } else if matches!(
            timestamp_format.date_visibility(),
            DateVisibility::HideDateToday | DateVisibility::HideDate
        ) {
            // Maybe this is a naked timestamp without any date?

            let tz = timestamp_format.to_jiff_time_zone();
            let today = jiff::Timestamp::now().to_zoned(tz).date();
            let today = today.strftime("%Y-%m-%d").to_string();

            if s.starts_with(&today) {
                None // prevent infinite recursion
            } else {
                let datetime_string = format!("{today}T{s}");
                Self::parse_with_format(&datetime_string, timestamp_format)
            }
        } else {
            None
        }
    }
}

impl Duration {
    /// Useful when showing dates/times on a timeline and you want it compact.
    ///
    /// When a duration is less than a second, we only show the time from the last whole second.
    pub fn format_subsecond_as_relative(self) -> String {
        let ns = self.as_nanos();

        let fractional_nanos = ns % 1_000_000_000;
        let is_whole_second = fractional_nanos == 0;

        if is_whole_second {
            self.to_string()
        } else {
            // We are in the sub-second resolution.
            // Showing the full time (HH:MM:SS.XXX or 3h 2m 6s …) becomes too long,
            // so instead we switch to showing the time as milliseconds since the last whole second:
            let ms = fractional_nanos as f64 * 1e-6;
            if fractional_nanos % 1_000_000 == 0 {
                format!("{ms:+.0} ms")
            } else if fractional_nanos % 100_000 == 0 {
                format!("{ms:+.1} ms")
            } else if fractional_nanos % 10_000 == 0 {
                format!("{ms:+.2} ms")
            } else if fractional_nanos % 1_000 == 0 {
                format!("{ms:+.3} ms")
            } else if fractional_nanos % 100 == 0 {
                format!("{ms:+.4} ms")
            } else if fractional_nanos % 10 == 0 {
                format!("{ms:+.5} ms")
            } else {
                format!("{ms:+.6} ms")
            }
        }
    }
}

// ------------------------------------------
// Duration ops

impl std::ops::Sub for Timestamp {
    type Output = Duration;

    #[inline]
    fn sub(self, rhs: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(rhs.0))
    }
}

impl std::ops::Add<Duration> for Timestamp {
    type Output = Self;

    #[inline]
    fn add(self, duration: Duration) -> Self::Output {
        Self(self.0.saturating_add(duration.as_nanos()))
    }
}

impl std::ops::AddAssign<Duration> for Timestamp {
    #[inline]
    fn add_assign(&mut self, duration: Duration) {
        self.0 = self.0.saturating_add(duration.as_nanos());
    }
}

impl std::ops::Sub<Duration> for Timestamp {
    type Output = Self;

    #[inline]
    fn sub(self, duration: Duration) -> Self::Output {
        Self(self.0.saturating_sub(duration.as_nanos()))
    }
}
