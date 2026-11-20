//! squ4t: everything that does not touch Windows.
//!
//! - [`ledger`]: store.py of omarchy-squat-break, ported so the same ledger file gives the same answers.
//! - [`pyjson`]: JSON read and written the way Python's `json` module does it.
//! - [`motion`]: the switch flight of 1nstall's motion.cs, and the one easing curve.
//! - [`look`]: the colours, after an unslop pass.

pub mod ledger;
pub mod look;
pub mod motion;
pub mod pyjson;
mod unicode_tables;
