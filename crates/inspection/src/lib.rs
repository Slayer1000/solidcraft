//! The neutral CMM program for SolidCraft's coordinate measuring plans.
//!
//! SolidCraft describes each setup's measurements as a
//! [`program::Program`] (elements, probing strategies, alignment, characteristics with their
//! parameters) and hands it to a **postprocessor**: a separate program, kept outside this
//! repository, that writes the plan of one measuring machine's software, and reads that
//! software's results back ([`results::Results`]). Nothing machine-specific lives here; how to
//! write a postprocessor: `docs/cmm-postprocessors.md`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod program;
pub mod results;
