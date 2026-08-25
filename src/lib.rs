//! Kiban is a storage engine, currently in phase 1: durable file
//! primitives.
//!
//! Implemented so far: atomic, durable publication of file contents
//! (`atomic`). See `docs/design/` for the decisions behind each component.

pub mod atomic;
