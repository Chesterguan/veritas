//! Policy rule types and configuration schema.
//!
//! A `PolicyConfig` is deserialized from TOML and holds an ordered list of
//! `PolicyRule`s.  Rules are evaluated in declaration order — the first
//! matching rule wins.  If no rule matches, the engine denies by default.

use serde::{Deserialize, Serialize};

/// The decision a rule produces when it matches an incoming `PolicyContext`.
///
/// Variants map directly to `PolicyVerdict` in veritas-contracts, but are
/// expressed as a plain string in TOML (kebab-case) for human readability.
///
/// Example in TOML:
/// ```toml
/// verdict = "allow"
/// verdict = "deny"
/// verdict = "require-approval"
/// verdict = "require-verification"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleVerdict {
    Allow,
    Deny,
    RequireApproval,
    RequireVerification,
}

/// A single policy rule loaded from TOML.
///
/// Rules are matched in the order they appear in the policy file.
/// The first rule whose `action` and `resource` patterns match the incoming
/// `PolicyContext` wins; subsequent rules are not evaluated.
///
/// Both `action` and `resource` support the special wildcard value `"*"`,
/// which matches any string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Stable identifier used in audit logs and error messages.
    pub id: String,

    /// Human-readable explanation of what this rule controls.
    pub description: String,

    /// The action pattern to match against `PolicyContext::action`.
    /// Use `"*"` to match any action.
    pub action: String,

    /// The resource pattern to match against `PolicyContext::resource`.
    /// Use `"*"` to match any resource.
    pub resource: String,

    /// Capability names that the agent MUST hold for this rule to produce its
    /// `verdict`.  If the agent lacks any listed capability, the engine denies
    /// the request regardless of `verdict`.  This is a defense-in-depth check:
    /// even an explicit `allow` rule cannot override a missing capability.
    #[serde(default)]
    pub required_capabilities: Vec<String>,

    /// The decision this rule produces when it matches and capabilities are met.
    pub verdict: RuleVerdict,

    /// Mandatory when `verdict = "deny"`.  Written to the audit log.
    pub deny_reason: Option<String>,

    /// Mandatory when `verdict = "require-approval"`.  Written to the audit log.
    pub approval_reason: Option<String>,

    /// Mandatory when `verdict = "require-approval"`.  Identifies the role
    /// (e.g. `"attending_physician"`) that must grant sign-off.
    pub approver_role: Option<String>,

    /// Mandatory when `verdict = "require-verification"`.  References the
    /// check identifier that the verifier will look up.
    pub verification_check_id: Option<String>,
}

impl PolicyRule {
    /// Return true if this rule matches the given `action` and `resource`.
    ///
    /// Matching logic:
    /// - `"*"` in the rule's `action` field matches any action string.
    /// - `"*"` in the rule's `resource` field matches any resource string.
    /// - Otherwise, both fields must match exactly (case-sensitive).
    pub fn matches(&self, action: &str, resource: &str) -> bool {
        let action_matches = self.action == "*" || self.action == action;
        let resource_matches = self.resource == "*" || self.resource == resource;
        action_matches && resource_matches
    }
}

/// The top-level structure deserialized from a TOML policy file.
///
/// Rules are evaluated in the order they appear in the `rules` array.
///
/// Example:
/// ```toml
/// [[rules]]
/// id = "allow-read-own-record"
/// description = "Patients may read their own records"
/// action = "read_patient_record"
/// resource = "*"
/// verdict = "allow"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Ordered list of rules.  First match wins.
    pub rules: Vec<PolicyRule>,
}
