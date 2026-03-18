//! Drift detection for registered models.
//!
//! [`InMemoryDriftMonitor`] is the reference implementation of the
//! [`DriftMonitor`] trait.  It tracks a rolling window of `confidence` scores
//! extracted from model invocation results and compares the current average to
//! a baseline established from the first full window of observations.
//!
//! # Design
//!
//! - Only the `confidence` field of the JSON result is observed.  Results that
//!   lack this field are silently ignored so non-confidence models can still be
//!   passed to `record` without errors.
//! - Interior mutability is provided by `std::sync::Mutex` — no async runtime
//!   is involved, consistent with the synchronous trusted path.
//! - Baseline is set once (from the first `window_size` observations) and is
//!   never updated thereafter.  Drift is defined as degradation relative to
//!   that fixed baseline.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use veritas_contracts::{error::VeritasResult, DriftMonitor, DriftStatus};

// ── DriftConfig ───────────────────────────────────────────────────────────────

/// Tuning parameters for [`InMemoryDriftMonitor`].
#[derive(Debug, Clone)]
pub struct DriftConfig {
    /// Confidence drop (baseline − current_avg) at which a `Warning` is issued.
    ///
    /// For example `0.1` means a 10 percentage-point decline triggers a warning.
    pub warning_threshold: f64,

    /// Confidence drop (baseline − current_avg) at which `Drifted` is reported.
    ///
    /// Must be ≥ `warning_threshold`.  A value of `0.2` flags any model whose
    /// average confidence has fallen by more than 20 pp relative to its baseline.
    pub drift_threshold: f64,

    /// Number of recent observations to keep in the rolling window.
    ///
    /// The baseline is set from the first complete window.
    pub window_size: usize,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            warning_threshold: 0.1,
            drift_threshold: 0.2,
            window_size: 10,
        }
    }
}

// ── InMemoryDriftMonitor ──────────────────────────────────────────────────────

/// Reference [`DriftMonitor`] that tracks per-model confidence scores in memory.
///
/// Thread-safe via `Mutex` guards on both internal maps.  Suitable for use in
/// a single-node VERITAS runtime; production deployments may replace this with
/// a persistent or distributed implementation.
pub struct InMemoryDriftMonitor {
    config: DriftConfig,
    /// Rolling window of confidence scores, keyed by model_id.
    records: Mutex<HashMap<String, VecDeque<f64>>>,
    /// Fixed baselines (average of first full window), keyed by model_id.
    baselines: Mutex<HashMap<String, f64>>,
}

impl InMemoryDriftMonitor {
    /// Construct a monitor with the supplied configuration.
    ///
    /// # Panics
    ///
    /// Panics if `config.window_size` is 0.
    pub fn new(config: DriftConfig) -> Self {
        assert!(
            config.window_size > 0,
            "DriftConfig::window_size must be > 0"
        );
        Self {
            config,
            records: Mutex::new(HashMap::new()),
            baselines: Mutex::new(HashMap::new()),
        }
    }
}

impl DriftMonitor for InMemoryDriftMonitor {
    fn record(&self, model_id: &str, result: &serde_json::Value) -> VeritasResult<()> {
        // Extract confidence — silently skip results that don't have it.
        let confidence = match result.get("confidence").and_then(|v| v.as_f64()) {
            Some(c) => c,
            None => return Ok(()),
        };

        let mut records = self.records.lock().expect("drift records lock poisoned");
        let window = records.entry(model_id.to_string()).or_default();

        // Enforce rolling window size.
        if window.len() == self.config.window_size {
            window.pop_front();
        }
        window.push_back(confidence);

        // Set baseline once the window is full for the first time.
        if window.len() == self.config.window_size {
            let mut baselines = self
                .baselines
                .lock()
                .expect("drift baselines lock poisoned");
            if !baselines.contains_key(model_id) {
                let avg = window.iter().sum::<f64>() / window.len() as f64;
                baselines.insert(model_id.to_string(), avg);
            }
        }

        Ok(())
    }

    fn check_drift(&self, model_id: &str) -> DriftStatus {
        let baselines = self
            .baselines
            .lock()
            .expect("drift baselines lock poisoned");
        let baseline = match baselines.get(model_id) {
            Some(&b) => b,
            // No baseline yet — not enough data to make a judgement.
            None => return DriftStatus::Stable,
        };

        let records = self.records.lock().expect("drift records lock poisoned");
        let window = match records.get(model_id) {
            Some(w) if !w.is_empty() => w,
            _ => return DriftStatus::Stable,
        };

        let current_avg = window.iter().sum::<f64>() / window.len() as f64;
        let drop = baseline - current_avg;

        if drop >= self.config.drift_threshold {
            DriftStatus::Drifted {
                metric: "confidence".to_string(),
                current: current_avg,
                threshold: self.config.drift_threshold,
            }
        } else if drop >= self.config.warning_threshold {
            DriftStatus::Warning {
                metric: "confidence".to_string(),
                current: current_avg,
                threshold: self.config.warning_threshold,
            }
        } else {
            DriftStatus::Stable
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DriftConfig, InMemoryDriftMonitor};
    use veritas_contracts::{DriftMonitor, DriftStatus};

    fn config() -> DriftConfig {
        DriftConfig {
            warning_threshold: 0.1,
            drift_threshold: 0.2,
            window_size: 5,
        }
    }

    fn monitor() -> InMemoryDriftMonitor {
        InMemoryDriftMonitor::new(config())
    }

    fn record_n(m: &InMemoryDriftMonitor, model_id: &str, confidence: f64, n: usize) {
        for _ in 0..n {
            m.record(model_id, &json!({ "confidence": confidence }))
                .unwrap();
        }
    }

    // ── 1: stable when confidence remains high ────────────────────────────────

    #[test]
    fn stable_when_confidence_does_not_drop() {
        let m = monitor();
        // Establish baseline at 0.90.
        record_n(&m, "model-a", 0.90, 5);
        // Continue at same level.
        record_n(&m, "model-a", 0.90, 3);

        assert_eq!(m.check_drift("model-a"), DriftStatus::Stable);
    }

    // ── 2: warning when confidence drops by warning_threshold ─────────────────

    #[test]
    fn warning_when_confidence_drops_to_warning_threshold() {
        let m = monitor();
        // Baseline: 0.90
        record_n(&m, "model-b", 0.90, 5);
        // Drop window to 0.79 (drop ≈ 0.11, crosses 0.10 warning threshold)
        record_n(&m, "model-b", 0.79, 5);

        match m.check_drift("model-b") {
            DriftStatus::Warning { metric, .. } => assert_eq!(metric, "confidence"),
            other => panic!("expected Warning, got {other:?}"),
        }
    }

    // ── 3: drifted when confidence drops by drift_threshold ───────────────────

    #[test]
    fn drifted_when_confidence_drops_to_drift_threshold() {
        let m = monitor();
        // Baseline: 0.90
        record_n(&m, "model-c", 0.90, 5);
        // Drop window to 0.65 (drop = 0.25, crosses 0.20 drift threshold)
        record_n(&m, "model-c", 0.65, 5);

        match m.check_drift("model-c") {
            DriftStatus::Drifted {
                metric,
                current,
                threshold,
            } => {
                assert_eq!(metric, "confidence");
                assert!((current - 0.65).abs() < 1e-9);
                assert!((threshold - 0.2).abs() < 1e-9);
            }
            other => panic!("expected Drifted, got {other:?}"),
        }
    }

    // ── 4: rolling window drops old data ─────────────────────────────────────

    #[test]
    fn window_rolls_and_old_data_is_discarded() {
        let m = monitor();
        // Establish baseline at 0.90 (window_size = 5, so 5 observations).
        record_n(&m, "model-d", 0.90, 5);

        // Now push 5 very-low scores — the window should roll, discarding the
        // earlier high scores entirely so the average reflects only the new ones.
        record_n(&m, "model-d", 0.50, 5);

        // Drop = 0.90 - 0.50 = 0.40, well above drift_threshold of 0.20.
        match m.check_drift("model-d") {
            DriftStatus::Drifted { .. } => {} // expected
            other => panic!("expected Drifted after window rolled; got {other:?}"),
        }
    }

    // ── 5: missing confidence field is silently ignored ───────────────────────

    #[test]
    fn missing_confidence_field_does_not_error() {
        let m = monitor();
        // Results with no confidence field must succeed without error.
        m.record("model-e", &json!({ "label": "benign" })).unwrap();
        m.record("model-e", &json!({})).unwrap();
        m.record("model-e", &json!({ "output": "some text" }))
            .unwrap();

        // No baseline can have been established, so status must be Stable.
        assert_eq!(m.check_drift("model-e"), DriftStatus::Stable);
    }

    // ── 6: zero window_size panics at construction ──────────────────────────

    #[test]
    #[should_panic(expected = "window_size must be > 0")]
    fn zero_window_size_panics() {
        let cfg = DriftConfig {
            window_size: 0,
            ..config()
        };
        InMemoryDriftMonitor::new(cfg);
    }

    // ── 7: unknown model returns Stable ──────────────────────────────────────

    #[test]
    fn unknown_model_returns_stable() {
        let m = monitor();
        assert_eq!(m.check_drift("never-seen"), DriftStatus::Stable);
    }
}
