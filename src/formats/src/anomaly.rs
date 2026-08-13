//! Structural oddities that are reported rather than repaired.
//!
//! An anomaly is what this crate says instead of quietly fixing something:
//! the bytes stay accounted for, the parse continues, and the report names
//! what was odd and where. Rejecting these inputs would refuse files the
//! client reads. Repairing them silently would hide a format fact.

use crate::reader::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomaly {
    /// Stable kebab-case identifier, asserted by conformance cases.
    pub kind: &'static str,
    pub span: Span,
    pub detail: String,
}

impl Anomaly {
    pub fn new(kind: &'static str, span: Span, detail: impl Into<String>) -> Self {
        Self {
            kind,
            span,
            detail: detail.into(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind,
            "span": self.span.to_json(),
            "detail": self.detail,
        })
    }
}
