//! Simulated healthcare data for the VERITAS reference runtime.
//!
//! All data in this module is hardcoded and fictional. No external systems are
//! contacted. This module acts as a stand-in for real clinical databases in a
//! production deployment.

use serde_json::{json, Value};

// ── Drug Interaction Database (mock) ─────────────────────────────────────────

/// Look up the interaction severity between two drugs.
///
/// Known pairs (order-independent):
/// - warfarin + aspirin         → HIGH
/// - metformin + contrast-dye   → HIGH
/// - lisinopril + potassium     → MEDIUM
/// - amoxicillin + ibuprofen    → LOW
///
/// Any other combination returns an UNKNOWN severity.
pub fn check_drug_interaction(drug_a: &str, drug_b: &str) -> Value {
    // Normalize to lowercase for case-insensitive matching.
    let a = drug_a.to_lowercase();
    let b = drug_b.to_lowercase();

    // Helper: order-independent pair match.
    let is_pair = |x: &str, y: &str| -> bool {
        (a == x && b == y) || (a == y && b == x)
    };

    let (severity, mechanism, recommendation) = if is_pair("warfarin", "aspirin") {
        (
            "HIGH",
            "Both agents inhibit platelet function and increase bleeding risk via distinct pathways",
            "Avoid concurrent use; if clinically necessary, monitor INR weekly and for signs of bleeding",
        )
    } else if is_pair("metformin", "contrast-dye") {
        (
            "HIGH",
            "Iodinated contrast may cause acute kidney injury, impairing metformin clearance and risking lactic acidosis",
            "Withhold metformin 48 hours before and after contrast administration; recheck renal function before resuming",
        )
    } else if is_pair("lisinopril", "potassium") {
        (
            "MEDIUM",
            "ACE inhibitors reduce aldosterone secretion, decreasing potassium excretion and risking hyperkalemia",
            "Monitor serum potassium within 1 week of starting or adjusting doses; avoid potassium supplements unless deficiency confirmed",
        )
    } else if is_pair("amoxicillin", "ibuprofen") {
        (
            "LOW",
            "NSAIDs may slightly reduce the renal clearance of amoxicillin at high doses",
            "Monitor for reduced antibiotic efficacy in patients with renal impairment; generally safe for short-term concurrent use",
        )
    } else {
        (
            "UNKNOWN",
            "No interaction data available for this drug pair in the reference database",
            "Consult a clinical pharmacist or full drug interaction database before co-prescribing",
        )
    };

    json!({
        "query": {
            "drug_a": drug_a,
            "drug_b": drug_b
        },
        "result": {
            "severity": severity,
            "mechanism": mechanism
        },
        "recommendation": recommendation
    })
}

// ── Clinical Notes (mock) ─────────────────────────────────────────────────────

/// Return two mock clinical notes for the given patient ID.
///
/// The notes contain realistic (but entirely fictional) clinical language.
/// No real patient identifiers or PHI are present.
pub fn get_patient_notes(patient_id: &str) -> Value {
    json!({
        "patient_id": patient_id,
        "notes": [
            {
                "note_id": "note-001",
                "date": "2026-02-10",
                "author": "Dr. A. Rivera",
                "department": "Internal Medicine",
                "text": "Patient presents with fatigue and mild dyspnea on exertion for the past three weeks. \
                         Vital signs stable. Heart rate 82 bpm, blood pressure 138/88 mmHg. \
                         SpO2 97% on room air. Lungs clear to auscultation bilaterally. \
                         EKG shows normal sinus rhythm without ischemic changes. \
                         CBC and BMP ordered. Patient advised to limit strenuous activity pending results."
            },
            {
                "note_id": "note-002",
                "date": "2026-02-14",
                "author": "Dr. A. Rivera",
                "department": "Internal Medicine",
                "text": "Follow-up visit. Lab results reviewed: hemoglobin 10.2 g/dL, consistent with mild anemia. \
                         Iron studies pending. BMP within normal limits. Renal function preserved (eGFR 74). \
                         Patient reports slight improvement in fatigue. \
                         Plan: start oral iron supplementation 325 mg daily, \
                         recheck CBC in four weeks. Referral to hematology if no improvement."
            }
        ]
    })
}

// ── Patient Symptoms (mock) ───────────────────────────────────────────────────

/// Return mock symptom data for the given patient ID.
///
/// Used by `SymptomAnalyzerAgent` in Scenario 4 (Clinical Decision Pipeline).
/// All data is fictional and hardcoded.
pub fn get_patient_symptoms(patient_id: &str) -> Value {
    json!({
        "patient_id": patient_id,
        "reported_symptoms": [
            { "symptom": "fatigue", "duration_days": 21, "severity": "moderate" },
            { "symptom": "dyspnea_on_exertion", "duration_days": 21, "severity": "mild" },
            { "symptom": "pallor", "duration_days": 14, "severity": "mild" }
        ],
        "vitals": {
            "heart_rate_bpm": 82,
            "blood_pressure": "138/88",
            "spo2_percent": 97,
            "temperature_c": 36.8
        },
        "recorded_date": "2026-02-18"
    })
}

// ── Patient Records (mock) ────────────────────────────────────────────────────

/// Return a mock patient record for the given patient ID.
///
/// The record includes conditions, current medications, and a consent flag that
/// controls whether AI-assisted queries are permitted.
///
/// Patients with IDs ending in "nc" (no-consent) have `ai_query_consent: false`.
pub fn get_patient_record(patient_id: &str) -> Value {
    let has_consent = !patient_id.ends_with("nc");

    json!({
        "patient_id": patient_id,
        "demographics": {
            "age": 58,
            "sex": "M",
            "primary_language": "English"
        },
        "conditions": [
            { "code": "E11.9",  "description": "Type 2 diabetes mellitus without complications" },
            { "code": "I10",    "description": "Essential hypertension" },
            { "code": "D50.9",  "description": "Iron deficiency anemia, unspecified" }
        ],
        "medications": [
            { "name": "Metformin",   "dose": "500 mg", "frequency": "twice daily" },
            { "name": "Lisinopril",  "dose": "10 mg",  "frequency": "once daily"  },
            { "name": "Ferrous sulfate", "dose": "325 mg", "frequency": "once daily" }
        ],
        "ai_query_consent": has_consent,
        "last_updated": "2026-02-14"
    })
}

// ── Insurance Coverage (mock) ─────────────────────────────────────────────────

/// Return mock insurance coverage data for the given procedure code.
///
/// Used by `InsuranceEligibilityAgent` in Scenario 5 (Prior Authorization).
/// Procedures whose code ends with "-uncovered" are not covered by the plan.
/// All data is fictional and hardcoded.
pub fn get_insurance_coverage(procedure_code: &str) -> Value {
    let covered = !procedure_code.ends_with("-uncovered");

    json!({
        "procedure_code": procedure_code,
        "covered": covered,
        "plan_name": if covered { json!("Blue Shield PPO") } else { serde_json::Value::Null },
        "copay_usd": if covered { json!(250) } else { serde_json::Value::Null },
        "requires_prior_auth": true,
        "checked_date": "2026-02-18"
    })
}
