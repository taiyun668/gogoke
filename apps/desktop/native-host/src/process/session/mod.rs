//! Design 37 native seat process boundary. No Node frame can select a token.

mod isolation;
mod compat_module;
mod credential_binding;
mod boot_identity;
mod legacy_holders_gone;

pub(crate) use isolation::{AppContainerProfile, IsolationError, SecurityCapabilities,
    LegacyAclInventory, LegacyHomeReceipt, LegacySourceReceipt,
    NativeCredentialAclRecoveryStep, adopt_holder_gone_source_baseline};
#[cfg(test)]
pub(crate) use isolation::holder_gone_acl_write_count_for_test;
pub(crate) use compat_module::{CompatModule, DirectoryRoots};
pub(crate) use credential_binding::{CredentialAlias, CredentialAliasScope,
    CredentialBinding, CredentialError};
pub(crate) use boot_identity::{NativeBootIdentity, NativeBootIdentityError};
pub(crate) use legacy_holders_gone::{native_boot_start, NativeLegacyHoldersGone,
    NativeLegacyHoldersGoneError, NativeProcessHoldersGone};
