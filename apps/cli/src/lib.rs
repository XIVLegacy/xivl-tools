//! Reusable CLI entry points used by the command and conformance runner.

pub mod extract;

use xivl_formats::{ErrorKind, FormatError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractFailureKind {
    Usage,
    Io,
    Format(ErrorKind),
    IncompleteResourceTriple,
    OffsetCountMismatch,
    OffsetDataLengthMismatch,
    EnableRowMismatch,
    InvalidResourceStructure,
    InvalidAttributeValue,
    RowIdOverflow,
}

impl ExtractFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage-error",
            Self::Io => "io-error",
            Self::Format(kind) => kind.as_str(),
            Self::IncompleteResourceTriple => "incomplete-resource-triple",
            Self::OffsetCountMismatch => "offset-count-mismatch",
            Self::OffsetDataLengthMismatch => "offset-data-length-mismatch",
            Self::EnableRowMismatch => "enable-row-mismatch",
            Self::InvalidResourceStructure => "invalid-resource-structure",
            Self::InvalidAttributeValue => "invalid-attribute-value",
            Self::RowIdOverflow => "row-id-overflow",
        }
    }

    pub fn is_parse(self) -> bool {
        !matches!(self, Self::Usage | Self::Io)
    }
}

#[derive(Debug)]
pub struct ExtractFailure {
    message: String,
    code: u8,
    kind: ExtractFailureKind,
    offset: Option<u64>,
}

impl ExtractFailure {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
            kind: ExtractFailureKind::Usage,
            offset: None,
        }
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
            kind: ExtractFailureKind::Io,
            offset: None,
        }
    }

    pub(crate) fn semantic(kind: ExtractFailureKind, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
            kind,
            offset: None,
        }
    }

    pub(crate) fn format(error: FormatError, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
            kind: ExtractFailureKind::Format(error.kind()),
            offset: Some(error.offset()),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn code(&self) -> u8 {
        self.code
    }

    pub fn kind(&self) -> ExtractFailureKind {
        self.kind
    }

    pub fn offset(&self) -> Option<u64> {
        self.offset
    }
}
