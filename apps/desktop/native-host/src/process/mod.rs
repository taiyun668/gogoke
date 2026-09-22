//! Windows process containment primitives for the public execution host.
//!
//! This module deliberately owns the kernel handles from suspended creation
//! through durable custody and activation. A caller cannot obtain a runnable
//! process before its custody callback succeeds.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
compile_error!("gogoke native process custody is Windows-only");
