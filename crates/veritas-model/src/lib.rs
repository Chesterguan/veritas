//! # veritas-model
//!
//! In-memory model registry for the VERITAS runtime.
//!
//! ## Overview
//!
//! `ModelRegistry` tracks every AI/ML model that has been explicitly registered
//! for use inside the VERITAS trust boundary.  Only models present in the
//! registry — and whose `ApprovalStatus` is `Approved` — should be permitted
//! by the policy engine.
//!
//! The registry is intentionally simple and synchronous.  There is no
//! persistence layer; callers populate it at startup from configuration or a
//! sealed manifest.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use veritas_model::{ModelRegistry, RegisteredModel};
//! use veritas_contracts::{ApprovalStatus, ModelModality, ModelProvenance};
//!
//! let mut registry = ModelRegistry::new();
//! registry.register(RegisteredModel {
//!     model_id: "acme/summariser-v2".to_string(),
//!     modality: ModelModality::TextToText,
//!     version: "2.1.0".to_string(),
//!     provenance: ModelProvenance {
//!         vendor: "Acme Medical AI".to_string(),
//!         training_data_hash: None,
//!         approval_status: ApprovalStatus::Approved,
//!         approved_by: Some("CMO".to_string()),
//!         approved_at: Some("2026-01-15T10:00:00Z".to_string()),
//!         regulatory_class: Some("non-clinical".to_string()),
//!     },
//! }).unwrap();
//!
//! assert!(registry.is_approved("acme/summariser-v2"));
//! ```

pub mod drift;

pub use drift::{DriftConfig, InMemoryDriftMonitor};

use std::collections::HashMap;

use veritas_contracts::{
    error::{VeritasError, VeritasResult},
    ApprovalStatus, DriftMonitor, DriftStatus, ModelDescriptor, ModelModality, ModelProvenance,
};

// ── RegisteredModel ───────────────────────────────────────────────────────────

/// A concrete `ModelDescriptor` that can be stored in the `ModelRegistry`.
///
/// Callers construct this struct directly and pass it to
/// `ModelRegistry::register`.  After registration the registry owns the entry;
/// use `ModelRegistry::get` to borrow it back.
#[derive(Debug, Clone)]
pub struct RegisteredModel {
    /// Stable, namespaced identifier (e.g. `"acme/summariser-v2"`).
    pub model_id: String,
    /// Input/output modality of this model.
    pub modality: ModelModality,
    /// Semantic version string.
    pub version: String,
    /// Provenance and approval metadata.
    pub provenance: ModelProvenance,
}

impl ModelDescriptor for RegisteredModel {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn modality(&self) -> &ModelModality {
        &self.modality
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn provenance(&self) -> &ModelProvenance {
        &self.provenance
    }
}

// ── ModelRegistry ─────────────────────────────────────────────────────────────

/// Central in-memory registry of models known to the VERITAS runtime.
///
/// The registry is the authoritative source of truth for:
/// - Which models exist (identity + provenance)
/// - Whether a model is currently approved for production use
/// - Which capability strings a model contributes to a `CapabilitySet`
///
/// # Deny-by-default
///
/// A model not present in the registry is treated the same as a revoked model —
/// the policy engine should reject any capability check for an unknown model_id.
#[derive(Debug, Default)]
pub struct ModelRegistry {
    /// Keyed by `model_id` for O(1) lookups.
    models: HashMap<String, RegisteredModel>,
}

impl ModelRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Register a model.
    ///
    /// # Errors
    ///
    /// Returns `VeritasError::InvalidInput` if a model with the same `model_id`
    /// is already registered.  Use `revoke` to change the status of an existing
    /// entry rather than re-registering.
    pub fn register(&mut self, model: RegisteredModel) -> VeritasResult<()> {
        if self.models.contains_key(&model.model_id) {
            return Err(VeritasError::InvalidInput {
                reason: format!(
                    "model '{}' is already registered; revoke it first if you need to replace it",
                    model.model_id
                ),
            });
        }
        self.models.insert(model.model_id.clone(), model);
        Ok(())
    }

    /// Returns `true` if the model is registered **and** its approval status is
    /// `ApprovalStatus::Approved`.  Any other status — including unknown models,
    /// `Pending`, `Revoked`, or `Experimental` — returns `false`.
    pub fn is_approved(&self, model_id: &str) -> bool {
        self.models
            .get(model_id)
            .map(|m| m.provenance.approval_status == ApprovalStatus::Approved)
            .unwrap_or(false)
    }

    /// Return a reference to the registered model descriptor, or `None` if the
    /// model is not in the registry.
    pub fn get(&self, model_id: &str) -> Option<&RegisteredModel> {
        self.models.get(model_id)
    }

    /// Revoke a registered model by setting its approval status to
    /// `ApprovalStatus::Revoked { reason }`.
    ///
    /// Revocation is intentionally permanent within a single registry instance —
    /// re-registration requires restarting with a fresh registry (or an
    /// explicit administrative override at the application layer).
    ///
    /// # Errors
    ///
    /// Returns `VeritasError::InvalidInput` if the model is not registered.
    pub fn revoke(&mut self, model_id: &str, reason: &str) -> VeritasResult<()> {
        let model = self
            .models
            .get_mut(model_id)
            .ok_or_else(|| VeritasError::InvalidInput {
                reason: format!("cannot revoke unknown model '{model_id}'"),
            })?;
        model.provenance.approval_status = ApprovalStatus::Revoked {
            reason: reason.to_string(),
        };
        Ok(())
    }

    /// Return all registered models whose modality matches `modality`.
    pub fn by_modality(&self, modality: &ModelModality) -> Vec<&RegisteredModel> {
        self.models
            .values()
            .filter(|m| &m.modality == modality)
            .collect()
    }

    /// Check the drift status of a model and auto-revoke it if `Drifted`.
    ///
    /// This integrates the drift monitor into the registry lifecycle: callers
    /// can poll this method after each batch of invocations to enforce automatic
    /// revocation without writing the policy-revoke logic in application code.
    ///
    /// # Returns
    ///
    /// The `DriftStatus` reported by `monitor`.  If the status is `Drifted`,
    /// the model's approval status is set to `Revoked` with a standard reason
    /// before returning.
    ///
    /// # Errors
    ///
    /// Returns `VeritasError::InvalidInput` if the model is not registered
    /// (consistent with `revoke`).
    pub fn check_and_update(
        &mut self,
        model_id: &str,
        monitor: &dyn DriftMonitor,
    ) -> VeritasResult<DriftStatus> {
        // Verify model exists before consulting the monitor.
        if !self.models.contains_key(model_id) {
            return Err(VeritasError::InvalidInput {
                reason: format!("cannot check drift for unknown model '{model_id}'"),
            });
        }

        let status = monitor.check_drift(model_id);

        if matches!(status, DriftStatus::Drifted { .. }) {
            self.revoke(model_id, "auto-revoked: model drift detected")?;
        }

        Ok(status)
    }

    /// Return all registered model IDs in an unspecified order.
    pub fn list(&self) -> Vec<&str> {
        self.models.keys().map(String::as_str).collect()
    }

    /// Generate capability strings for integration with `CapabilitySet`.
    ///
    /// Returns:
    /// - `"model:<model_id>"` — always present when the model is registered.
    /// - `"model:approved"` — present only when `is_approved` is `true`.
    ///
    /// Returns an empty `Vec` for an unknown `model_id`.
    pub fn capabilities_for(&self, model_id: &str) -> Vec<String> {
        match self.models.get(model_id) {
            None => vec![],
            Some(model) => {
                let mut caps = vec![format!("model:{}", model.model_id)];
                if model.provenance.approval_status == ApprovalStatus::Approved {
                    caps.push("model:approved".to_string());
                }
                caps
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use veritas_contracts::{ApprovalStatus, ModelModality, ModelProvenance};

    use super::{ModelRegistry, RegisteredModel};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn approved_provenance(vendor: &str) -> ModelProvenance {
        ModelProvenance {
            vendor: vendor.to_string(),
            training_data_hash: None,
            approval_status: ApprovalStatus::Approved,
            approved_by: Some("CMO".to_string()),
            approved_at: Some("2026-01-15T10:00:00Z".to_string()),
            regulatory_class: Some("non-clinical".to_string()),
        }
    }

    fn make_model(id: &str, modality: ModelModality, status: ApprovalStatus) -> RegisteredModel {
        RegisteredModel {
            model_id: id.to_string(),
            modality,
            version: "1.0.0".to_string(),
            provenance: ModelProvenance {
                vendor: "Test Vendor".to_string(),
                training_data_hash: None,
                approval_status: status,
                approved_by: None,
                approved_at: None,
                regulatory_class: None,
            },
        }
    }

    // ── 1: register and retrieve ──────────────────────────────────────────────

    #[test]
    fn register_and_get_returns_model() {
        let mut registry = ModelRegistry::new();
        let model = RegisteredModel {
            model_id: "acme/summariser-v2".to_string(),
            modality: ModelModality::TextToText,
            version: "2.1.0".to_string(),
            provenance: approved_provenance("Acme Medical AI"),
        };

        registry.register(model).unwrap();

        let retrieved = registry.get("acme/summariser-v2").unwrap();
        assert_eq!(retrieved.model_id, "acme/summariser-v2");
        assert_eq!(retrieved.version, "2.1.0");
        assert_eq!(retrieved.modality, ModelModality::TextToText);
        assert_eq!(retrieved.provenance.vendor, "Acme Medical AI");
    }

    #[test]
    fn get_unknown_model_returns_none() {
        let registry = ModelRegistry::new();
        assert!(registry.get("does-not-exist").is_none());
    }

    // ── 2: duplicate registration fails ──────────────────────────────────────

    #[test]
    fn register_duplicate_model_id_fails() {
        let mut registry = ModelRegistry::new();
        let model = make_model(
            "chest-xray-v3.2",
            ModelModality::ImageToLabel,
            ApprovalStatus::Approved,
        );

        registry.register(model.clone()).unwrap();
        let result = registry.register(model);

        assert!(result.is_err(), "duplicate registration must fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("chest-xray-v3.2"),
            "error should name the duplicate model id"
        );
    }

    // ── 3: is_approved status variants ───────────────────────────────────────

    #[test]
    fn is_approved_returns_true_for_approved() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "m1",
                ModelModality::TextToText,
                ApprovalStatus::Approved,
            ))
            .unwrap();
        assert!(registry.is_approved("m1"));
    }

    #[test]
    fn is_approved_returns_false_for_pending() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "m2",
                ModelModality::TextToText,
                ApprovalStatus::Pending,
            ))
            .unwrap();
        assert!(!registry.is_approved("m2"));
    }

    #[test]
    fn is_approved_returns_false_for_experimental() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "m3",
                ModelModality::TextToText,
                ApprovalStatus::Experimental,
            ))
            .unwrap();
        assert!(!registry.is_approved("m3"));
    }

    #[test]
    fn is_approved_returns_false_for_revoked() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "m4",
                ModelModality::TextToText,
                ApprovalStatus::Revoked {
                    reason: "pre-revoked".to_string(),
                },
            ))
            .unwrap();
        assert!(!registry.is_approved("m4"));
    }

    #[test]
    fn is_approved_returns_false_for_unknown_model() {
        let registry = ModelRegistry::new();
        assert!(!registry.is_approved("unknown-model"));
    }

    // ── 4: revoke changes status and is_approved returns false ────────────────

    #[test]
    fn revoke_approved_model_makes_is_approved_false() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "chest-xray-v3.2",
                ModelModality::ImageToLabel,
                ApprovalStatus::Approved,
            ))
            .unwrap();

        assert!(
            registry.is_approved("chest-xray-v3.2"),
            "should be approved before revocation"
        );

        registry
            .revoke("chest-xray-v3.2", "safety recall issued by manufacturer")
            .unwrap();

        assert!(
            !registry.is_approved("chest-xray-v3.2"),
            "must not be approved after revocation"
        );

        // Verify the revocation reason is persisted in the provenance.
        let model = registry.get("chest-xray-v3.2").unwrap();
        match &model.provenance.approval_status {
            ApprovalStatus::Revoked { reason } => {
                assert!(reason.contains("safety recall"), "reason must be stored");
            }
            other => panic!("expected Revoked, got {other:?}"),
        }
    }

    // ── 5: by_modality filters correctly ─────────────────────────────────────

    #[test]
    fn by_modality_returns_only_matching_models() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "text-a",
                ModelModality::TextToText,
                ApprovalStatus::Approved,
            ))
            .unwrap();
        registry
            .register(make_model(
                "text-b",
                ModelModality::TextToText,
                ApprovalStatus::Pending,
            ))
            .unwrap();
        registry
            .register(make_model(
                "image-a",
                ModelModality::ImageToLabel,
                ApprovalStatus::Approved,
            ))
            .unwrap();

        let text_models = registry.by_modality(&ModelModality::TextToText);
        assert_eq!(text_models.len(), 2, "should return both TextToText models");

        let image_models = registry.by_modality(&ModelModality::ImageToLabel);
        assert_eq!(
            image_models.len(),
            1,
            "should return only the ImageToLabel model"
        );
        assert_eq!(image_models[0].model_id, "image-a");

        let tabular_models = registry.by_modality(&ModelModality::TabularToScore);
        assert!(
            tabular_models.is_empty(),
            "no TabularToScore models registered"
        );
    }

    // ── 6: capabilities_for returns correct strings ───────────────────────────

    #[test]
    fn capabilities_for_approved_includes_model_approved() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "chest-xray-v3.2",
                ModelModality::ImageToLabel,
                ApprovalStatus::Approved,
            ))
            .unwrap();

        let caps = registry.capabilities_for("chest-xray-v3.2");

        assert!(
            caps.contains(&"model:chest-xray-v3.2".to_string()),
            "must include model-scoped capability"
        );
        assert!(
            caps.contains(&"model:approved".to_string()),
            "must include model:approved for approved model"
        );
        assert_eq!(caps.len(), 2);
    }

    #[test]
    fn capabilities_for_revoked_omits_model_approved() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "chest-xray-v3.2",
                ModelModality::ImageToLabel,
                ApprovalStatus::Approved,
            ))
            .unwrap();
        registry.revoke("chest-xray-v3.2", "safety recall").unwrap();

        let caps = registry.capabilities_for("chest-xray-v3.2");

        assert!(
            caps.contains(&"model:chest-xray-v3.2".to_string()),
            "model-scoped capability must still be present"
        );
        assert!(
            !caps.contains(&"model:approved".to_string()),
            "model:approved must be absent for a revoked model"
        );
        assert_eq!(caps.len(), 1);
    }

    #[test]
    fn capabilities_for_pending_omits_model_approved() {
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "pending-model",
                ModelModality::TextToText,
                ApprovalStatus::Pending,
            ))
            .unwrap();

        let caps = registry.capabilities_for("pending-model");
        assert!(!caps.contains(&"model:approved".to_string()));
        assert_eq!(caps.len(), 1);
    }

    #[test]
    fn capabilities_for_unknown_model_returns_empty() {
        let registry = ModelRegistry::new();
        let caps = registry.capabilities_for("does-not-exist");
        assert!(caps.is_empty());
    }

    // ── 7: revoke non-existent model fails ───────────────────────────────────

    #[test]
    fn revoke_unknown_model_returns_error() {
        let mut registry = ModelRegistry::new();
        let result = registry.revoke("ghost-model", "never existed");

        assert!(result.is_err(), "revoking unknown model must fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ghost-model"),
            "error message should name the unknown model id"
        );
    }

    // ── 8: check_and_update auto-revokes a drifted model ─────────────────────

    #[test]
    fn check_and_update_auto_revokes_drifted_model() {
        use crate::drift::{DriftConfig, InMemoryDriftMonitor};
        use veritas_contracts::{DriftMonitor, DriftStatus};

        let monitor = InMemoryDriftMonitor::new(DriftConfig {
            warning_threshold: 0.1,
            drift_threshold: 0.2,
            window_size: 5,
        });

        // Register an approved model.
        let mut registry = ModelRegistry::new();
        registry
            .register(make_model(
                "drift-model",
                ModelModality::TextToText,
                ApprovalStatus::Approved,
            ))
            .unwrap();
        assert!(registry.is_approved("drift-model"));

        // Establish baseline at 0.90.
        for _ in 0..5 {
            monitor
                .record("drift-model", &serde_json::json!({ "confidence": 0.90 }))
                .unwrap();
        }
        // Drop confidence to 0.60 (drop = 0.30 > drift_threshold of 0.20).
        for _ in 0..5 {
            monitor
                .record("drift-model", &serde_json::json!({ "confidence": 0.60 }))
                .unwrap();
        }

        let status = registry.check_and_update("drift-model", &monitor).unwrap();

        // Status must be Drifted.
        assert!(
            matches!(status, DriftStatus::Drifted { .. }),
            "expected Drifted, got {status:?}"
        );
        // Model must have been revoked.
        assert!(
            !registry.is_approved("drift-model"),
            "drifted model must be auto-revoked"
        );
        match &registry
            .get("drift-model")
            .unwrap()
            .provenance
            .approval_status
        {
            ApprovalStatus::Revoked { reason } => {
                assert!(
                    reason.contains("drift"),
                    "revocation reason must mention drift"
                );
            }
            other => panic!("expected Revoked, got {other:?}"),
        }
    }

    #[test]
    fn check_and_update_unknown_model_returns_error() {
        use crate::drift::{DriftConfig, InMemoryDriftMonitor};

        let monitor = InMemoryDriftMonitor::new(DriftConfig::default());
        let mut registry = ModelRegistry::new();

        let result = registry.check_and_update("ghost-model", &monitor);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ghost-model"));
    }

    // ── list ──────────────────────────────────────────────────────────────────

    #[test]
    fn list_returns_all_registered_model_ids() {
        let mut registry = ModelRegistry::new();
        assert!(registry.list().is_empty());

        registry
            .register(make_model(
                "alpha",
                ModelModality::TextToText,
                ApprovalStatus::Approved,
            ))
            .unwrap();
        registry
            .register(make_model(
                "beta",
                ModelModality::ImageToText,
                ApprovalStatus::Pending,
            ))
            .unwrap();

        let ids: std::collections::HashSet<&str> = registry.list().into_iter().collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("alpha"));
        assert!(ids.contains("beta"));
    }
}
