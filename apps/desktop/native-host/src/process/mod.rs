//! Windows process containment primitives for the public execution host.
//!
//! This module owns kernel handles from suspended creation through activation.
//! Activation is restricted to trusted in-crate service composition; the
//! production service commits its coordination custody row before calling it.
//! These process primitives do not themselves prove a database commit or a
//! RootLock held by an arbitrary Rust caller.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
compile_error!("gogoke native process custody is Windows-only");
