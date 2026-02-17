//! Capability-based access control types.
//!
//! VERITAS uses a capability model: an agent may only take an action if it
//! holds the corresponding capability. Capabilities are granted at startup
//! and are never elevated at runtime — this is a hard security invariant.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// An opaque capability token.
///
/// Capability names should be namespaced and descriptive:
/// e.g. "phi:read", "phi:write", "order:submit", "audit:write".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability(pub String);

impl Capability {
    /// Construct a capability from any string-like value.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// The full set of capabilities granted to an agent execution.
///
/// This is constructed at startup by the hosting application and passed
/// to the executor. The executor checks it before calling `agent.propose()`.
#[derive(Debug, Clone, Default)]
pub struct CapabilitySet {
    inner: HashSet<Capability>,
}

impl CapabilitySet {
    /// Grant a capability to this set.
    pub fn grant(&mut self, capability: Capability) {
        self.inner.insert(capability);
    }

    /// Return true if the set contains the given capability.
    pub fn has(&self, capability: &Capability) -> bool {
        self.inner.contains(capability)
    }

    /// Return an iterator over all granted capabilities.
    pub fn all(&self) -> impl Iterator<Item = &Capability> {
        self.inner.iter()
    }
}
