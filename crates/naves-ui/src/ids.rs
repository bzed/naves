//! Minimal identifier types ported from `re_log_types` 0.38.1 (`StoreId`, `EntryId`)
//! and `re_uri` 0.38.1 (`Origin`, `Scheme`) — <https://github.com/rerun-io/rerun>,
//! MIT OR Apache-2.0.
//!
//! Only the API surface used by `re_ui`'s `command` module is kept: these are
//! opaque, cheaply clonable identifiers with `Debug + Eq + Hash`. All storage /
//! gRPC machinery of the originals (arrow serialization, `re_tuid`, …) is removed.

// ----------------------------------------------------------------------------
// StoreId

/// What kind of store a [`StoreId`] refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StoreKind {
    /// A recording of data.
    Recording,

    /// A blueprint (layout / configuration).
    Blueprint,
}

impl std::fmt::Display for StoreKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recording => f.write_str("recording"),
            Self::Blueprint => f.write_str("blueprint"),
        }
    }
}

/// The application a recording belongs to, e.g. `rerun example`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApplicationId(pub String);

impl ApplicationId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ApplicationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies a single recording within an application.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordingId(pub String);

impl RecordingId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RecordingId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unique identifier of a recording or blueprint store.
///
/// Simplified port of `re_log_types::StoreId`: same shape, plain `String` ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StoreId {
    pub kind: StoreKind,
    pub application_id: ApplicationId,
    pub recording_id: RecordingId,
}

impl StoreId {
    pub fn new(
        kind: StoreKind,
        application_id: impl Into<String>,
        recording_id: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            application_id: ApplicationId(application_id.into()),
            recording_id: RecordingId(recording_id.into()),
        }
    }

    pub fn kind(&self) -> StoreKind {
        self.kind
    }

    pub fn application_id(&self) -> &ApplicationId {
        &self.application_id
    }

    pub fn recording_id(&self) -> &RecordingId {
        &self.recording_id
    }

    pub fn is_recording(&self) -> bool {
        self.kind == StoreKind::Recording
    }

    pub fn is_blueprint(&self) -> bool {
        self.kind == StoreKind::Blueprint
    }
}

impl std::fmt::Display for StoreId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}({}/{})",
            self.kind, self.application_id.0, self.recording_id.0
        )
    }
}

// ----------------------------------------------------------------------------
// EntryId

/// Unique identifier of an entry (dataset or table) on a Redap server.
///
/// Simplified port of `re_log_types::EntryId`: the original wraps
/// `re_tuid::Tuid`; here it is a 128-bit id generated from a time/counter mix
/// (session-unique is all the UI needs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId {
    pub id: u128,
}

impl EntryId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = web_time::SystemTime::now()
            .duration_since(web_time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

        Self {
            id: ((nanos as u128) << 64) ^ (seq.wrapping_mul(0x9E3779B97F4A7C15) as u128),
        }
    }
}

impl std::fmt::Display for EntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032X}", self.id)
    }
}

// ----------------------------------------------------------------------------
// Origin (re_uri)

/// The scheme of a rerun server address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    #[default]
    RerunHttp,
    RerunHttps,
}

impl std::fmt::Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RerunHttp => write!(f, "rerun+http"),
            Self::RerunHttps => write!(f, "rerun+https"),
        }
    }
}

/// The address of a rerun server: scheme, host and port.
///
/// Simplified port of `crate::ids::Origin` (same shape; the original additionally
/// carries gRPC/arrow-serialization support).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Origin {
    pub scheme: Scheme,
    pub host: url::Host<String>,
    pub port: u16,
}

impl Origin {
    pub fn from_scheme_host_port(scheme: Scheme, host: url::Host<String>, port: u16) -> Self {
        Self { scheme, host, port }
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}:{}", self.scheme, self.host, self.port)
    }
}
