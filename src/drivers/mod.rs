//! Hardware driver backends.
//!
//! Each backend module implements scanner-specific open/scan/close behavior.
//! The top-level `driver` registry selects one backend at runtime.

pub mod digitalpersona;
