//! VERITAS Healthcare Reference Runtime — Demo CLI
//!
//! Runs one or all of the three healthcare demo scenarios.  Each scenario uses
//! real VERITAS components (policy engine, audit writer, verifier, executor)
//! wired together with mock clinical data.
//!
//! Usage:
//!   cargo run -p demo -- run-all
//!   cargo run -p demo -- drug-interaction
//!   cargo run -p demo -- note-summarizer
//!   cargo run -p demo -- patient-query

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use veritas_ref_healthcare::scenarios::{drug_interaction, note_summarizer, patient_query};

// ── CLI definition ────────────────────────────────────────────────────────────

/// VERITAS — Policy-bound AI runtime healthcare demo.
///
/// Each subcommand runs one or all of the three clinical AI scenarios,
/// demonstrating VERITAS's policy, capability, and verification enforcement.
#[derive(Parser)]
#[command(
    name = "demo",
    about = "VERITAS healthcare reference runtime demo",
    long_about = "Runs VERITAS healthcare demo scenarios showing policy enforcement,\n\
                  capability checks, output verification, and audit chain integrity."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run all three healthcare scenarios in sequence.
    RunAll,
    /// Scenario 1: Drug Interaction Checker (capability-gated DB query).
    DrugInteraction,
    /// Scenario 2: Clinical Note Summarizer (PII custom verifier rule).
    NoteSummarizer,
    /// Scenario 3: Patient Data Query (Allow / CapabilityMissing / Deny).
    PatientQuery,
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    // Initialize structured logging.  Set RUST_LOG=debug for verbose output.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    print_banner();

    let result = match cli.command {
        Command::RunAll => run_all(),
        Command::DrugInteraction => run_drug_interaction(),
        Command::NoteSummarizer => run_note_summarizer(),
        Command::PatientQuery => run_patient_query(),
    };

    match result {
        Ok(()) => {
            println!("All selected scenarios completed successfully.");
        }
        Err(e) => {
            eprintln!("Demo error: {}", e);
            std::process::exit(1);
        }
    }
}

// ── Scenario dispatch ─────────────────────────────────────────────────────────

fn run_all() -> veritas_contracts::error::VeritasResult<()> {
    run_drug_interaction()?;
    run_note_summarizer()?;
    run_patient_query()?;
    Ok(())
}

fn run_drug_interaction() -> veritas_contracts::error::VeritasResult<()> {
    drug_interaction::run_scenario()
}

fn run_note_summarizer() -> veritas_contracts::error::VeritasResult<()> {
    note_summarizer::run_scenario()
}

fn run_patient_query() -> veritas_contracts::error::VeritasResult<()> {
    patient_query::run_scenario()
}

// ── Banner ────────────────────────────────────────────────────────────────────

fn print_banner() {
    println!();
    println!("VERITAS — Policy-bound AI Runtime");
    println!("Healthcare Reference Demo");
    println!("=================================");
    println!();
    println!("VERITAS enforcement pipeline per step:");
    println!("  [1] Policy engine evaluates (action, resource) → Allow / Deny / RequireApproval");
    println!("  [2] Capability check: agent must hold all declared capabilities");
    println!("  [3] Agent propose() called — ONLY after steps 1 & 2 pass");
    println!("  [4] Verifier checks output against JSON Schema + semantic rules");
    println!("  [5] State transition + immutable audit record written to SHA-256 chain");
    println!();
}
