//! Model capability types for the VERITAS runtime.
//!
//! This module defines the contracts for integrating AI/ML models as first-class
//! capabilities inside the VERITAS trust boundary. Models are untrusted by
//! default — every invocation must be policy-gated and its output verified
//! before the result is accepted by the runtime.
//!
//! # Design choices
//!
//! - `ModelCapability` uses `serde_json::Value` for input/output so the trait
//!   is object-safe and can be stored as `Box<dyn ModelCapability>`.
//! - Timestamps are `String` (ISO-8601) rather than `chrono::DateTime` to
//!   keep the dependency surface minimal.
//! - `ApprovalStatus::Revoked` carries a `reason` field to satisfy audit
//!   trail requirements without a separate lookup.

use serde::{Deserialize, Serialize};

use crate::error::VeritasResult;

// ── Modality ─────────────────────────────────────────────────────────────────

/// The input/output modality of a model.
///
/// This is used by the policy engine to restrict which modalities are
/// permitted for a given agent action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelModality {
    /// Standard text-in / text-out (LLM, summariser, …).
    TextToText,
    /// Image (or document scan) in, natural-language description out.
    ImageToText,
    /// Image in, categorical label out (classifier, detector).
    ImageToLabel,
    /// Structured tabular data in, numeric score out (risk model).
    TabularToScore,
    /// Time-series signal in, alert or annotation out.
    TimeSeriesToAlert,
    /// Multiple input modalities combined.
    MultiModal,
    /// Any modality not covered by the variants above.
    Custom(String),
}

// ── ApprovalStatus ───────────────────────────────────────────────────────────

/// Regulatory / organisational approval lifecycle state of a model.
///
/// The policy engine may deny invocations of models that are not `Approved`
/// or that have been `Revoked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ApprovalStatus {
    /// Model has passed all required reviews and is cleared for production use.
    Approved,
    /// Review is in progress; not yet cleared for production.
    Pending,
    /// Previously approved but approval has been withdrawn.
    Revoked {
        /// Human-readable explanation recorded in the audit trail.
        reason: String,
    },
    /// Available for research or development but not production patient care.
    Experimental,
}

// ── ModelProvenance ──────────────────────────────────────────────────────────

/// Provenance metadata attached to every model descriptor.
///
/// This struct captures the chain-of-custody information regulators and
/// auditors need to reconstruct how a model was selected, reviewed, and
/// approved for use in a specific clinical context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProvenance {
    /// The organisation or individual that produced and maintains the model.
    pub vendor: String,

    /// Optional SHA-256 (or similar) hash of the training dataset manifest,
    /// enabling downstream verification that the model was not retrained on
    /// unreviewed data.
    pub training_data_hash: Option<String>,

    /// Current approval lifecycle state.
    pub approval_status: ApprovalStatus,

    /// Role or identity of the person/committee who approved the model.
    pub approved_by: Option<String>,

    /// ISO-8601 timestamp string at which approval was granted.
    /// Stored as `String` to avoid pulling in `chrono` as a hard dependency.
    pub approved_at: Option<String>,

    /// Regulatory classification (e.g. "FDA 510(k)", "CE Class IIa", "non-clinical").
    pub regulatory_class: Option<String>,
}

// ── TokenUsage ───────────────────────────────────────────────────────────────

/// Token consumption reported by a language-model invocation.
///
/// Useful for cost tracking and for audit entries that must record resource
/// usage alongside the logical result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Tokens consumed from the prompt / context window.
    pub input_tokens: u64,
    /// Tokens produced by the model in its response.
    pub output_tokens: u64,
}

// ── ModelResult ──────────────────────────────────────────────────────────────

/// The structured result returned by a `ModelCapability` invocation.
///
/// Wrapping raw model output in `ModelResult` ensures that provenance
/// metadata (which model, which version, how long it took) travels with the
/// output through the audit and verification pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResult<T> {
    /// The raw model output, typed by the caller.
    pub output: T,

    /// Confidence score in `[0.0, 1.0]`, if the model provides one.
    pub confidence: Option<f64>,

    /// Wall-clock time from invocation start to response receipt, in ms.
    pub latency_ms: u64,

    /// Token usage counters; `None` for non-language models.
    pub token_usage: Option<TokenUsage>,

    /// Identifier of the model that produced this result.
    pub model_id: String,

    /// Version string of the model that produced this result.
    pub model_version: String,
}

// ── ModelDescriptor trait ────────────────────────────────────────────────────

/// Read-only identity and provenance surface of a model.
///
/// Implementing this trait is required before a model can be wrapped in a
/// `ModelCapability`. The descriptor is consulted by the policy engine and
/// stored in every audit entry.
pub trait ModelDescriptor: Send + Sync {
    /// A stable, namespaced identifier for the model
    /// (e.g. `"acme/summariser-v2"` or `"openai/gpt-4o"`).
    fn model_id(&self) -> &str;

    /// The input/output modality of this model.
    fn modality(&self) -> &ModelModality;

    /// Semantic version string (e.g. `"2.1.0"` or `"2024-05-13"`).
    fn version(&self) -> &str;

    /// Provenance and approval metadata for this model.
    fn provenance(&self) -> &ModelProvenance;
}

// ── ModelCapability trait ────────────────────────────────────────────────────

/// The primary integration point for AI/ML models inside the VERITAS runtime.
///
/// Implementors wrap a concrete model backend and expose it through this
/// object-safe interface. The runtime treats every `ModelCapability` as
/// untrusted: inputs are validated before invocation and outputs are verified
/// by the `Verifier` stage before being consumed by the next agent step.
///
/// # Object safety
///
/// Both `invoke` and `validate_input` use `serde_json::Value` so that
/// `Box<dyn ModelCapability>` is valid without requiring `dyn Any` hacks or
/// generic associated types.
pub trait ModelCapability: Send + Sync {
    /// Return the descriptor (identity + provenance) of this model.
    fn descriptor(&self) -> &dyn ModelDescriptor;

    /// Invoke the model with a JSON-serialised input.
    ///
    /// The implementation is responsible for:
    /// 1. Calling `validate_input` (or the policy engine will do so first).
    /// 2. Recording any internal errors as `VeritasError` variants.
    /// 3. Returning a `ModelResult` that includes timing and token metadata.
    fn invoke(
        &self,
        input: &serde_json::Value,
    ) -> VeritasResult<ModelResult<serde_json::Value>>;

    /// Validate the input schema and semantic constraints before invocation.
    ///
    /// Returns `Ok(())` if the input is acceptable, or a `VeritasError` with
    /// a descriptive reason otherwise. This is called by the policy engine
    /// during the Capability stage before `invoke` is permitted to run.
    fn validate_input(&self, input: &serde_json::Value) -> VeritasResult<()>;
}

// ── DriftStatus ──────────────────────────────────────────────────────────────

/// The drift state of a model as assessed by a `DriftMonitor`.
///
/// The policy engine can use this to gate further invocations: a `Drifted`
/// model may be automatically revoked while a `Warning` model may trigger
/// human review without immediate revocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DriftStatus {
    /// Model behaviour is within acceptable bounds relative to the baseline.
    Stable,
    /// Confidence has dropped enough to warrant attention but not full revocation.
    Warning {
        /// The metric that triggered the warning (e.g. `"confidence"`).
        metric: String,
        /// The current value of the metric.
        current: f64,
        /// The threshold that was crossed.
        threshold: f64,
    },
    /// Confidence has dropped below the drift threshold; model should be revoked.
    Drifted {
        /// The metric that triggered the drift flag (e.g. `"confidence"`).
        metric: String,
        /// The current value of the metric.
        current: f64,
        /// The threshold that was crossed.
        threshold: f64,
    },
}

// ── DriftMonitor trait ────────────────────────────────────────────────────────

/// Hook for monitoring model behaviour over time.
///
/// Implementations observe invocation results and expose a `check_drift` query
/// so the registry can decide whether to auto-revoke a degraded model.
///
/// # Design
///
/// - `record` takes `&self` so the trait is object-safe with interior mutability.
/// - Only the `confidence` field is used by the reference implementation; other
///   metrics can be added by custom implementations without changing the trait.
pub trait DriftMonitor: Send + Sync {
    /// Record a model invocation result for drift analysis.
    ///
    /// Implementations should extract any relevant metrics from `result` and
    /// update internal state.  If `result` does not contain a recognisable
    /// metric the call must succeed silently (never propagate a parse error).
    fn record(&self, model_id: &str, result: &serde_json::Value) -> VeritasResult<()>;

    /// Check whether the model has drifted beyond acceptable bounds.
    ///
    /// Returns `DriftStatus::Stable` when no baseline has been established yet
    /// or when there is insufficient data in the rolling window.
    fn check_drift(&self, model_id: &str) -> DriftStatus;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ApprovalStatus construction ───────────────────────────────────────────

    #[test]
    fn approval_status_approved_round_trip() {
        let status = ApprovalStatus::Approved;
        let json = serde_json::to_string(&status).unwrap();
        let decoded: ApprovalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded);
    }

    #[test]
    fn approval_status_pending_round_trip() {
        let status = ApprovalStatus::Pending;
        let json = serde_json::to_string(&status).unwrap();
        let decoded: ApprovalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded);
    }

    #[test]
    fn approval_status_revoked_carries_reason() {
        let status = ApprovalStatus::Revoked {
            reason: "model exhibited demographic bias in validation study".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        let decoded: ApprovalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded);

        // Ensure the reason survives the round-trip.
        if let ApprovalStatus::Revoked { reason } = decoded {
            assert!(reason.contains("demographic bias"));
        } else {
            panic!("expected Revoked variant");
        }
    }

    #[test]
    fn approval_status_experimental_round_trip() {
        let status = ApprovalStatus::Experimental;
        let json = serde_json::to_string(&status).unwrap();
        let decoded: ApprovalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded);
    }

    // ── ModelProvenance construction ──────────────────────────────────────────

    #[test]
    fn model_provenance_approved_full() {
        let prov = ModelProvenance {
            vendor: "Acme Medical AI".to_string(),
            training_data_hash: Some(
                "sha256:abc123def456abc123def456abc123def456abc123def456abc123def456abcd"
                    .to_string(),
            ),
            approval_status: ApprovalStatus::Approved,
            approved_by: Some("Chief Medical Officer".to_string()),
            approved_at: Some("2025-11-01T09:00:00Z".to_string()),
            regulatory_class: Some("FDA 510(k)".to_string()),
        };

        assert_eq!(prov.vendor, "Acme Medical AI");
        assert_eq!(prov.approval_status, ApprovalStatus::Approved);
        assert!(prov.training_data_hash.is_some());
        assert!(prov.regulatory_class.is_some());
    }

    #[test]
    fn model_provenance_minimal_experimental() {
        let prov = ModelProvenance {
            vendor: "Research Lab".to_string(),
            training_data_hash: None,
            approval_status: ApprovalStatus::Experimental,
            approved_by: None,
            approved_at: None,
            regulatory_class: None,
        };

        assert_eq!(prov.approval_status, ApprovalStatus::Experimental);
        assert!(prov.training_data_hash.is_none());
        assert!(prov.approved_by.is_none());
    }

    #[test]
    fn model_provenance_round_trip() {
        let prov = ModelProvenance {
            vendor: "Acme Medical AI".to_string(),
            training_data_hash: Some("sha256:deadbeef".to_string()),
            approval_status: ApprovalStatus::Approved,
            approved_by: Some("CMO".to_string()),
            approved_at: Some("2025-11-01T09:00:00Z".to_string()),
            regulatory_class: Some("CE Class IIa".to_string()),
        };

        let json = serde_json::to_string(&prov).unwrap();
        let decoded: ModelProvenance = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.vendor, prov.vendor);
        assert_eq!(decoded.approval_status, prov.approval_status);
        assert_eq!(decoded.training_data_hash, prov.training_data_hash);
        assert_eq!(decoded.approved_by, prov.approved_by);
        assert_eq!(decoded.approved_at, prov.approved_at);
        assert_eq!(decoded.regulatory_class, prov.regulatory_class);
    }

    // ── ModelResult construction ──────────────────────────────────────────────

    #[test]
    fn model_result_construction_with_all_fields() {
        let result = ModelResult {
            output: serde_json::json!({ "summary": "Patient presents with chest pain." }),
            confidence: Some(0.91),
            latency_ms: 342,
            token_usage: Some(TokenUsage {
                input_tokens: 1024,
                output_tokens: 128,
            }),
            model_id: "acme/summariser-v2".to_string(),
            model_version: "2.1.0".to_string(),
        };

        assert_eq!(result.model_id, "acme/summariser-v2");
        assert_eq!(result.latency_ms, 342);
        assert!(result.confidence.unwrap() > 0.9);
        let usage = result.token_usage.unwrap();
        assert_eq!(usage.input_tokens, 1024);
        assert_eq!(usage.output_tokens, 128);
    }

    #[test]
    fn model_result_construction_minimal() {
        let result: ModelResult<serde_json::Value> = ModelResult {
            output: serde_json::json!({"label": "benign"}),
            confidence: None,
            latency_ms: 55,
            token_usage: None,
            model_id: "acme/classifier-v1".to_string(),
            model_version: "1.0.0".to_string(),
        };

        assert!(result.confidence.is_none());
        assert!(result.token_usage.is_none());
        assert_eq!(result.latency_ms, 55);
    }

    // ── ModelModality equality ────────────────────────────────────────────────

    #[test]
    fn model_modality_equality() {
        assert_eq!(ModelModality::TextToText, ModelModality::TextToText);
        assert_eq!(ModelModality::ImageToText, ModelModality::ImageToText);
        assert_ne!(ModelModality::TextToText, ModelModality::ImageToText);
        assert_ne!(ModelModality::TabularToScore, ModelModality::TimeSeriesToAlert);
    }

    #[test]
    fn model_modality_custom_equality() {
        let a = ModelModality::Custom("ecg-to-rhythm".to_string());
        let b = ModelModality::Custom("ecg-to-rhythm".to_string());
        let c = ModelModality::Custom("mri-to-segment".to_string());

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn model_modality_round_trip() {
        let modalities = vec![
            ModelModality::TextToText,
            ModelModality::ImageToText,
            ModelModality::ImageToLabel,
            ModelModality::TabularToScore,
            ModelModality::TimeSeriesToAlert,
            ModelModality::MultiModal,
            ModelModality::Custom("ecg-to-rhythm".to_string()),
        ];

        for modality in modalities {
            let json = serde_json::to_string(&modality).unwrap();
            let decoded: ModelModality = serde_json::from_str(&json).unwrap();
            assert_eq!(modality, decoded);
        }
    }
}
