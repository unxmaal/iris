//! Phase 3.3: snapshot determinism validator.
//!
//! Loads a saved snapshot twice, runs the CPU `n` instructions inline (with
//! all peripheral threads stopped to eliminate scheduling jitter), and
//! diffs the resulting CPU state digests. Two passes over the same starting
//! state should produce bit-identical CPU registers — any divergence points
//! at non-determinism in `load_snapshot` (host wallclock leakage, missing
//! `load_state` field, uninitialised structure) that would silently corrupt
//! CI replays.
//!
//! With JIT descoped (Phase 2.5), the original "JIT vs interp lockstep"
//! framing is gone; this is the snapshot-determinism portion. Peripheral
//! threads are stopped during the test so device-side timing variance
//! doesn't leak into the result.

use crate::machine::Machine;
use crate::mips_exec::CpuStateDigest;

/// Result of `validate_snapshot_determinism`.
#[derive(Debug)]
pub struct DeterminismReport {
    pub instructions_run: u64,
    pub deterministic: bool,
    /// Per-field divergence list. Empty when `deterministic` is true.
    pub diffs: Vec<(String, String, String)>,
    pub state_a: CpuStateDigest,
    pub state_b: CpuStateDigest,
}

impl DeterminismReport {
    pub fn summary(&self) -> String {
        if self.deterministic {
            format!(
                "deterministic for {} instructions (PC=0x{:016x})",
                self.instructions_run, self.state_a.pc
            )
        } else {
            let mut s = format!(
                "DIVERGED after {} instructions ({} field(s)):",
                self.instructions_run,
                self.diffs.len()
            );
            for (field, a, b) in &self.diffs {
                s.push_str(&format!("\n  {}: A={} B={}", field, a, b));
            }
            s
        }
    }
}

/// Run two passes of `load_snapshot(name); step n; capture` and diff the
/// resulting CPU state digests. Side effects: leaves the machine stopped
/// after both passes, with the second-pass state loaded. Caller is
/// responsible for any subsequent `start`/`restart_peripherals`.
pub fn validate_snapshot_determinism(
    machine: &mut Machine,
    name: &str,
    n_instructions: u64,
) -> Result<DeterminismReport, String> {
    // Pass A: load with everything paused → step inline → capture.
    // load_snapshot_paused leaves CPU and peripheral threads stopped, so no
    // thread runs between load and digest. This is the key to surfacing
    // genuine load_state determinism issues vs. thread-scheduling jitter.
    machine.load_snapshot_paused(name)?;
    let executed_a = machine.cpu_step_n_inline(n_instructions)?;
    let state_a = machine.cpu_state_digest()?;

    // Pass B: same starting snapshot, fresh load.
    machine.load_snapshot_paused(name)?;
    let executed_b = machine.cpu_step_n_inline(n_instructions)?;
    let state_b = machine.cpu_state_digest()?;

    if executed_a != executed_b {
        return Err(format!(
            "step counts disagree: A ran {}, B ran {} (CPU stopped itself differently)",
            executed_a, executed_b
        ));
    }

    let diffs = state_a.diff(&state_b);
    Ok(DeterminismReport {
        instructions_run: executed_a,
        deterministic: diffs.is_empty(),
        diffs,
        state_a,
        state_b,
    })
}
