//! Reusable conditional auto-translate lookup. See docs/formats/fixed-control.md.

use std::io::Read;
use std::path::Path;

pub use crate::fixed_phrase::Locale;
pub use crate::initial_registry::Resolver;

pub(crate) const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveErrorKind {
    Usage,
    Io,
    Parse,
}

#[derive(Debug)]
pub struct ResolveError {
    message: String,
    kind: ResolveErrorKind,
}

impl ResolveError {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ResolveErrorKind::Usage,
        }
    }

    pub(crate) fn parse(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ResolveErrorKind::Parse,
        }
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ResolveErrorKind::Io,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn kind(&self) -> ResolveErrorKind {
        self.kind
    }
    pub fn code(&self) -> u8 {
        if self.kind == ResolveErrorKind::Parse {
            2
        } else {
            1
        }
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResolveError {}

pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, ResolveError> {
    let failed =
        |error: String| ResolveError::io(format!("cannot read '{}': {error}", path.display()));
    let file = std::fs::File::open(path).map_err(|error| failed(error.to_string()))?;
    let mut data = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| failed(error.to_string()))?;
    if data.len() as u64 > MAX_INPUT_BYTES {
        return Err(failed(format!(
            "input is larger than the {MAX_INPUT_BYTES}-byte limit"
        )));
    }
    Ok(data)
}
