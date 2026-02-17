//! Output verification schema and report types.
//!
//! Before any agent output is delivered or used to advance state, the
//! verifier runs it against an `OutputSchema`. Only a passing
//! `VerificationReport` allows the step to proceed.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The full specification the verifier checks agent outputs against.
///
/// Schemas are defined at runtime startup and passed to the Executor.
/// They combine a JSON Schema document with additional business-logic rules
/// that go beyond what JSON Schema can express.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSchema {
    /// Unique identifier for this schema (e.g. "patient-intake-v1").
    pub schema_id: String,
    /// A JSON Schema document used for structural validation.
    pub json_schema: Value,
    /// Additional domain rules evaluated after structural validation.
    pub rules: Vec<VerificationRule>,
}

/// A single verification rule applied to an agent output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRule {
    /// Unique identifier for this rule, referenced in failure reports.
    pub rule_id: String,
    /// Human-readable description for audit logs and operator tooling.
    pub description: String,
    /// The verification logic to apply.
    pub rule_type: VerificationRuleType,
}

/// The kinds of verification checks VERITAS supports out of the box.
///
/// `Custom` allows domain adapters to hook in arbitrary logic by name,
/// keeping the core verifier free of healthcare-specific knowledge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationRuleType {
    /// The field at `field_path` must be present and non-null.
    RequiredField {
        /// JSONPath-style dotted path, e.g. "patient.id".
        field_path: String,
    },

    /// The field at `field_path` must equal one of `allowed`.
    AllowedValues {
        /// JSONPath-style dotted path.
        field_path: String,
        /// The exhaustive list of permitted values.
        allowed: Vec<Value>,
    },

    /// The field at `field_path` must not match `pattern` (regex or substring).
    ForbiddenPattern {
        /// JSONPath-style dotted path.
        field_path: String,
        /// The forbidden pattern (implementation-defined matching semantics).
        pattern: String,
    },

    /// Delegate to a named custom function registered by the hosting application.
    Custom {
        /// Name of the registered function.
        function_name: String,
    },
}

/// The result of running all rules in an `OutputSchema` against an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    /// True only if all rules passed.
    pub passed: bool,
    /// All failures collected during this verification run. Empty on pass.
    pub failures: Vec<VerificationFailure>,
}

/// A single rule failure within a `VerificationReport`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationFailure {
    /// The `rule_id` of the rule that failed.
    pub rule_id: String,
    /// Human-readable explanation of why the rule failed.
    pub message: String,
}
