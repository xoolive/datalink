//! Optional I/Q demodulators for aviation datalink bearers.
//!
//! This module is available only when the crate is built with
//! `features = ["demod"]`. It contains native demodulators for VDL Mode 2
//! (`vdl2`), classic VHF ACARS (`vhf`), and experimental HFDL (`hfdl`), plus
//! resampling helpers used by the `datalink` CLI frontends.
//!
//! Parser-only users should keep the default feature set and use
//! [`crate::decode`] directly.

pub mod hfdl;
pub mod resample;
pub mod vdl2;
pub mod vhf;
