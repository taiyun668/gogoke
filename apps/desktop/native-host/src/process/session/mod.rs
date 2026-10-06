//! Design 37 native seat process boundary. No Node frame can select a token.

mod isolation;
mod compat_module;
mod credential_binding;
mod boot_identity;
mod legacy_holders_gone;

pub(crate) use isolation::{AppContainerProfile, IsolationError, SecurityCapabilities,
    LegacyAclInventory, LegacyHomeReceipt, LegacySourceReceipt,
    NativeCredentialAclRecoveryStep, adopt_holder_gone_source_baseline};
pub(crate) use isolation::{GrokAuthMetadata, GrokHomeObject, GrokAclSnapshot, grok_root_acl,
    grok_residue_acl, observe_grok_auth,
    grant_grok_home_root, grant_grok_auth, verify_grok_home_tree,
    verify_grok_auth, revoke_grok_home_root, revoke_grok_auth,
    inspect_grok_home_residue, revoke_grok_home_residue};
#[cfg(test)]
pub(crate) use isolation::holder_gone_acl_write_count_for_test;
pub(crate) use compat_module::{CompatModule, DirectoryRoots};
pub(crate) use credential_binding::{CredentialAlias, CredentialAliasScope,
    CredentialBinding, CredentialError};
pub(crate) use boot_identity::{NativeBootIdentity, NativeBootIdentityError};
pub(crate) use legacy_holders_gone::{native_boot_start, NativeLegacyHoldersGone,
    NativeLegacyHoldersGoneError, NativeProcessHoldersGone};
