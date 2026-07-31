//! ESC/POS printing abstraction (plan section 6).
//!
//! Serves both non-fiscal receipts and DANFCE (NFC-e printed representation)
//! through the same pipeline, plus cash-drawer "kick" pulses sent through
//! the printer's RJ11/RJ12 connector.
//!
//! Not implemented in Phase 0 (Foundation) — later phases (1, 6) wire this
//! up to a real ESC/POS backend (e.g. the `escpos` crate or a direct
//! `serialport` implementation).

/// Abstraction over a physical receipt printer, so business logic never
/// depends on a specific model/protocol.
#[allow(dead_code)]
pub trait Printer {
    fn print_receipt(&self, content: &[u8]) -> Result<(), String>;
    fn open_cash_drawer(&self) -> Result<(), String>;
}

// TODO(phase 1): non-fiscal receipt rendering + ESC/POS driver.
// TODO(phase 6): DANFCE rendering (58mm/80mm) once fiscal emission exists.
