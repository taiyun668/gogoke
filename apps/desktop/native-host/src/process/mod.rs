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
mod session;

#[cfg(windows)]
pub use windows::*;
#[cfg(all(windows, test))]
pub(crate) use session::holder_gone_acl_write_count_for_test;
#[cfg(windows)]
pub(crate) use session::{AppContainerProfile, CompatModule, DirectoryRoots, IsolationError,
    CredentialAlias, CredentialAliasScope, CredentialBinding, CredentialError,
    NativeBootIdentity, NativeBootIdentityError,
    native_boot_start, NativeLegacyHoldersGone, NativeLegacyHoldersGoneError,
    LegacyAclInventory, LegacyHomeReceipt, LegacySourceReceipt,
    NativeProcessHoldersGone, NativeCredentialAclRecoveryStep,
    adopt_holder_gone_source_baseline};

#[cfg(not(windows))]
compile_error!("gogoke native process custody is Windows-only");
