//! Report structural anomalies without repairing the input.
//!
//! Parsing continues while every byte remains accounted for. Each anomaly records
//! its location so callers can inspect irregularities in files the client reads.

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
