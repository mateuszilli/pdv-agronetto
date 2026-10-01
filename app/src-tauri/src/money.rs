//! Money crosses the Tauri invoke boundary as integer cents (see
//! app/src/lib/money.ts) so cart/total arithmetic on the frontend never
//! touches floating point. SQLite's NUMERIC columns have no fixed-point
//! type, so on the way in/out of storage cents are converted to/from a
//! plain decimal `f64` — a single divide/multiply at the boundary, not a
//! repeated calculation, so it doesn't reintroduce the rounding problem
//! integer cents exists to avoid.

pub fn cents_to_amount(cents: i64) -> f64 {
    cents as f64 / 100.0
}

pub fn amount_to_cents(amount: f64) -> i64 {
    (amount * 100.0).round() as i64
}
