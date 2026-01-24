//! IEC 61131-3 standard function blocks.
//!
//! This crate provides standard function blocks per the IEC 61131-3 specification:
//!
//! - **Timers** ([`timers`]): TON, TOF, TP
//! - **Counters** ([`counters`]): CTU, CTD, CTUD
//! - **Triggers** ([`triggers`]): R_TRIG, F_TRIG
//! - **Bistable** ([`bistable`]): SR, RS
//!
//! # Example
//!
//! ```
//! use plc_stdlib::timers::Ton;
//! use plc_stdlib::counters::Ctu;
//! use plc_stdlib::triggers::RTrig;
//! use plc_stdlib::bistable::Sr;
//!
//! // Timer on-delay
//! let mut ton = Ton::new();
//! let (q, et) = ton.call(true, 1_000_000_000, 100_000_000);
//!
//! // Counter up
//! let mut ctu = Ctu::new();
//! let (q, cv) = ctu.call(true, false, 10);
//!
//! // Rising edge trigger
//! let mut rtrig = RTrig::new();
//! let edge = rtrig.call(true);
//!
//! // Set-reset flip-flop
//! let mut sr = Sr::new();
//! let q = sr.call(true, false);
//! ```

pub mod bistable;
pub mod counters;
pub mod timers;
pub mod triggers;

// Re-export main types for convenience
pub use bistable::{Rs, Sr};
pub use counters::{Ctd, Ctu, Ctud};
pub use timers::{Tof, Ton, Tp};
pub use triggers::{FTrig, RTrig};
