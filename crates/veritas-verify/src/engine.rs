//! Schema-based output verifier for the VERITAS runtime.
//!
//! `SchemaVerifier` implements the `Verifier` trait from `veritas-core`.
//! Verification runs in two phases:
//!
//! 1. **Structural** — the `AgentOutput` payload is validated against the
//!    `OutputSchema::json_schema` document using the `jsonschema` crate.
//! 2. **Semantic** — each `VerificationRule` in `OutputSchema::rules` is
//!    evaluated in order.  All failures are collected before returning so
//!    operators see the full failure set in one pass.
//!
//! Custom rules delegate to named functions registered via `register_rule`.
//! Keeping healthcare-specific logic out of the core verifier is a VERITAS
//! design principle — domain adapters register what they need.

use std::collections::HashMap;

use tracing::{debug, warn};

use veritas_contracts::{
    agent::AgentOutput,
    error::VeritasResult,
    verify::{
        OutputSchema, VerificationFailure, VerificationReport, VerificationRuleType,
    },
};
use veritas_core::traits::Verifier;

/// A caller-supplied verification function.
///
/// Receives the full `AgentOutput` payload.  Returns `Some(message)` when the
/// check fails with a human-readable explanation, or `None` on success.
pub type CustomVerifierFn = Box<dyn Fn(&serde_json::Value) -> Option<String> + Send + Sync>;

/// The VERITAS output verifier.
///
/// Combines JSON Schema structural validation with a set of semantic rules.
/// Custom rules can be registered at startup by the hosting application —
/// this keeps healthcare-specific knowledge out of the trusted runtime core.
pub struct SchemaVerifier {
    /// Named custom verification functions provided by domain adapters.
    custom_rules: HashMap<String, CustomVerifierFn>,
}

impl SchemaVerifier {
    /// Create a verifier with no custom rules registered.
    pub fn new() -> Self {
        Self {
            custom_rules: HashMap::new(),
        }
    }

    /// Register a custom verification function under `name`.
    ///
    /// The name must match the `function_name` field used in
    /// `VerificationRuleType::Custom` rules. Registering the same name twice
    /// replaces the previous function.
    pub fn register_rule(&mut self, name: impl Into<String>, f: CustomVerifierFn) {
        self.custom_rules.insert(name.into(), f);
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Resolve a dot-notation field path (e.g. `"patient.id"`) against a JSON
    /// value.  Returns `None` when any segment is missing or the value is JSON
    /// `null`.
    fn resolve_path<'v>(value: &'v serde_json::Value, path: &str) -> Option<&'v serde_json::Value> {
        let mut current = value;
        for segment in path.split('.') {
            match current.get(segment) {
                Some(v) if !v.is_null() => current = v,
                _ => return None,
            }
        }
        Some(current)
    }
}

impl Default for SchemaVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Verifier for SchemaVerifier {
    /// Verify `output` against `schema`.
    ///
    /// Runs structural JSON Schema validation first, then evaluates every
    /// semantic rule.  All failures are accumulated — the caller receives the
    /// full picture in one report rather than only the first failure.
    fn verify(
        &self,
        output: &AgentOutput,
        schema: &OutputSchema,
    ) -> VeritasResult<VerificationReport> {
        let mut failures: Vec<VerificationFailure> = Vec::new();
        let payload = &output.payload;

        // ── Phase 1: JSON Schema structural validation ────────────────────────
        //
        // A null json_schema means "no structural constraint" — skip validation.
        // This matches how the executor tests construct a bare OutputSchema.
        if !schema.json_schema.is_null() {
            match jsonschema::validator_for(&schema.json_schema) {
                Ok(validator) => {
                    for error in validator.iter_errors(payload) {
                        let message = format!(
                            "JSON Schema violation at {}: {}",
                            error.instance_path, error
                        );
                        warn!(schema_id = %schema.schema_id, %message, "structural validation failure");
                        failures.push(VerificationFailure {
                            rule_id: "json-schema".to_string(),
                            message,
                        });
                    }
                }
                Err(e) => {
                    // A malformed schema document is a configuration error; treat
                    // it as a single structural failure so the run can still be
                    // audited rather than crashing the executor.
                    let message = format!("invalid JSON Schema document: {e}");
                    warn!(schema_id = %schema.schema_id, %message, "schema compilation failure");
                    failures.push(VerificationFailure {
                        rule_id: "json-schema".to_string(),
                        message,
                    });
                }
            }
        }

        // ── Phase 2: Semantic rule evaluation ────────────────────────────────
        for rule in &schema.rules {
            debug!(
                rule_id = %rule.rule_id,
                description = %rule.description,
                "evaluating verification rule"
            );

            let failure_msg: Option<String> = match &rule.rule_type {
                // ── RequiredField ─────────────────────────────────────────────
                // The field must be present at the resolved path and non-null.
                VerificationRuleType::RequiredField { field_path } => {
                    if Self::resolve_path(payload, field_path).is_none() {
                        Some(format!("required field '{field_path}' is missing or null"))
                    } else {
                        None
                    }
                }

                // ── AllowedValues ─────────────────────────────────────────────
                // The field value must appear in the exhaustive allowed set.
                VerificationRuleType::AllowedValues { field_path, allowed } => {
                    match Self::resolve_path(payload, field_path) {
                        None => Some(format!(
                            "field '{field_path}' is missing; cannot check allowed values"
                        )),
                        Some(actual) => {
                            if allowed.contains(actual) {
                                None
                            } else {
                                Some(format!(
                                    "field '{field_path}' has value {actual} which is not in the allowed set"
                                ))
                            }
                        }
                    }
                }

                // ── ForbiddenPattern ──────────────────────────────────────────
                // The field string value must not contain the forbidden pattern
                // as a substring.  Non-string fields pass silently — the rule is
                // only meaningful for string values.
                VerificationRuleType::ForbiddenPattern { field_path, pattern } => {
                    match Self::resolve_path(payload, field_path) {
                        None => None, // field absent — nothing to check
                        Some(v) => {
                            if let Some(s) = v.as_str() {
                                if s.contains(pattern.as_str()) {
                                    Some(format!(
                                        "field '{field_path}' contains forbidden pattern '{pattern}'"
                                    ))
                                } else {
                                    None
                                }
                            } else {
                                None // non-string value — rule does not apply
                            }
                        }
                    }
                }

                // ── Custom ────────────────────────────────────────────────────
                // Delegate to the registered function. An unregistered name is
                // itself a failure so misconfigured rules surface immediately.
                VerificationRuleType::Custom { function_name } => {
                    match self.custom_rules.get(function_name.as_str()) {
                        Some(f) => f(payload),
                        None => Some(format!(
                            "no custom rule registered for function name '{function_name}'"
                        )),
                    }
                }
            };

            if let Some(message) = failure_msg {
                warn!(
                    rule_id = %rule.rule_id,
                    %message,
                    "semantic rule failed"
                );
                failures.push(VerificationFailure {
                    rule_id: rule.rule_id.clone(),
                    message,
                });
            }
        }

        let passed = failures.is_empty();
        debug!(
            schema_id = %schema.schema_id,
            passed,
            failure_count = failures.len(),
            "verification complete"
        );

        Ok(VerificationReport { passed, failures })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use veritas_contracts::{
        agent::AgentOutput,
        verify::{OutputSchema, VerificationRule, VerificationRuleType},
    };

    use super::SchemaVerifier;
    use veritas_core::traits::Verifier;

    // ── Builder helpers ───────────────────────────────────────────────────────

    fn make_output(payload: serde_json::Value) -> AgentOutput {
        AgentOutput {
            kind: "response".to_string(),
            payload,
        }
    }

    fn make_schema(json_schema: serde_json::Value, rules: Vec<VerificationRule>) -> OutputSchema {
        OutputSchema {
            schema_id: "test-schema-v1".to_string(),
            json_schema,
            rules,
        }
    }

    fn rule(id: &str, desc: &str, rule_type: VerificationRuleType) -> VerificationRule {
        VerificationRule {
            rule_id: id.to_string(),
            description: desc.to_string(),
            rule_type,
        }
    }

    // ── JSON Schema tests ─────────────────────────────────────────────────────

    /// A payload that satisfies the JSON Schema must produce passed: true with
    /// no failures when no semantic rules are configured.
    #[test]
    fn test_schema_pass() {
        let verifier = SchemaVerifier::new();

        // Schema: object with a required string field "status".
        let json_schema = json!({
            "type": "object",
            "properties": {
                "status": { "type": "string" }
            },
            "required": ["status"]
        });

        let output = make_output(json!({ "status": "ok" }));
        let schema = make_schema(json_schema, vec![]);

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(report.passed, "expected pass, failures: {:?}", report.failures);
        assert!(report.failures.is_empty());
    }

    /// A payload missing a field declared required by the JSON Schema must
    /// produce passed: false.
    #[test]
    fn test_schema_fail() {
        let verifier = SchemaVerifier::new();

        let json_schema = json!({
            "type": "object",
            "properties": {
                "status": { "type": "string" }
            },
            "required": ["status"]
        });

        // Payload is missing "status".
        let output = make_output(json!({ "other_field": 42 }));
        let schema = make_schema(json_schema, vec![]);

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(!report.passed, "expected failure for missing required field");
        assert!(!report.failures.is_empty());
        assert_eq!(report.failures[0].rule_id, "json-schema");
    }

    // ── RequiredField tests ───────────────────────────────────────────────────

    /// A payload containing the required field at the given dot-path passes.
    #[test]
    fn test_required_field_pass() {
        let verifier = SchemaVerifier::new();

        let output = make_output(json!({ "patient": { "id": "p-001" } }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "req-patient-id",
                "patient.id must be present",
                VerificationRuleType::RequiredField {
                    field_path: "patient.id".to_string(),
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(report.passed, "expected pass, failures: {:?}", report.failures);
    }

    /// A payload missing the required field path produces a failure that
    /// references the correct rule_id.
    #[test]
    fn test_required_field_fail() {
        let verifier = SchemaVerifier::new();

        // No "patient" key at all.
        let output = make_output(json!({ "other": "value" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "req-patient-id",
                "patient.id must be present",
                VerificationRuleType::RequiredField {
                    field_path: "patient.id".to_string(),
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(!report.passed);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].rule_id, "req-patient-id");
        assert!(
            report.failures[0].message.contains("patient.id"),
            "failure message should name the missing field: {}",
            report.failures[0].message
        );
    }

    // ── AllowedValues tests ───────────────────────────────────────────────────

    /// When the field value is in the allowed set the rule passes.
    #[test]
    fn test_allowed_values_pass() {
        let verifier = SchemaVerifier::new();

        let output = make_output(json!({ "status": "approved" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "allowed-status",
                "status must be approved or pending",
                VerificationRuleType::AllowedValues {
                    field_path: "status".to_string(),
                    allowed: vec![json!("approved"), json!("pending")],
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(report.passed, "expected pass, failures: {:?}", report.failures);
    }

    /// When the field value is outside the allowed set the rule fails.
    #[test]
    fn test_allowed_values_fail() {
        let verifier = SchemaVerifier::new();

        let output = make_output(json!({ "status": "rejected" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "allowed-status",
                "status must be approved or pending",
                VerificationRuleType::AllowedValues {
                    field_path: "status".to_string(),
                    allowed: vec![json!("approved"), json!("pending")],
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(!report.passed);
        assert_eq!(report.failures[0].rule_id, "allowed-status");
    }

    // ── ForbiddenPattern tests ────────────────────────────────────────────────

    /// A string field containing the forbidden substring causes a failure.
    #[test]
    fn test_forbidden_pattern_detected() {
        let verifier = SchemaVerifier::new();

        let output = make_output(json!({ "notes": "patient SSN: 123-45-6789 recorded" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "no-ssn",
                "output must not contain SSN patterns",
                VerificationRuleType::ForbiddenPattern {
                    field_path: "notes".to_string(),
                    pattern: "SSN".to_string(),
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(!report.passed);
        assert_eq!(report.failures[0].rule_id, "no-ssn");
        assert!(
            report.failures[0].message.contains("SSN"),
            "failure should name the forbidden pattern: {}",
            report.failures[0].message
        );
    }

    // ── Custom rule tests ─────────────────────────────────────────────────────

    /// A registered custom function that returns None causes the rule to pass.
    #[test]
    fn test_custom_rule_pass() {
        let mut verifier = SchemaVerifier::new();
        verifier.register_rule(
            "always-pass",
            Box::new(|_payload| None),
        );

        let output = make_output(json!({ "field": "value" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "custom-check",
                "delegate to always-pass function",
                VerificationRuleType::Custom {
                    function_name: "always-pass".to_string(),
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(report.passed, "expected pass, failures: {:?}", report.failures);
    }

    /// A registered custom function that returns Some(msg) causes a failure
    /// with the rule_id of the enclosing rule.
    #[test]
    fn test_custom_rule_fail() {
        let mut verifier = SchemaVerifier::new();
        verifier.register_rule(
            "always-fail",
            Box::new(|_payload| Some("custom check failed: condition not met".to_string())),
        );

        let output = make_output(json!({ "field": "value" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "custom-check",
                "delegate to always-fail function",
                VerificationRuleType::Custom {
                    function_name: "always-fail".to_string(),
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(!report.passed);
        assert_eq!(report.failures[0].rule_id, "custom-check");
        assert!(
            report.failures[0].message.contains("condition not met"),
            "failure should carry the message from the custom function: {}",
            report.failures[0].message
        );
    }

    /// Referencing a custom function name that was never registered is itself
    /// a failure — misconfigured schemas must surface immediately.
    #[test]
    fn test_unregistered_custom_rule() {
        let verifier = SchemaVerifier::new(); // no rules registered

        let output = make_output(json!({ "field": "value" }));
        let schema = make_schema(
            serde_json::Value::Null,
            vec![rule(
                "phantom-check",
                "references a function that does not exist",
                VerificationRuleType::Custom {
                    function_name: "does-not-exist".to_string(),
                },
            )],
        );

        let report = verifier.verify(&output, &schema).unwrap();

        assert!(!report.passed);
        assert_eq!(report.failures[0].rule_id, "phantom-check");
        assert!(
            report.failures[0].message.contains("does-not-exist"),
            "failure should name the missing function: {}",
            report.failures[0].message
        );
    }
}
