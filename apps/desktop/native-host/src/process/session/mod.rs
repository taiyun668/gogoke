//! Design 37 native seat process boundary. No Node frame can select a token.

mod isolation;
mod compat_module;
mod credential_binding;
mod boot_identity;

pub(crate) use isolation::{AppContainerProfile, IsolationError, SecurityCapabilities,
    LegacyAclInventory, LegacyHomeReceipt, LegacySourceReceipt};
pub(crate) use compat_module::{CompatModule, DirectoryRoots};
pub(crate) use credential_binding::{CredentialAlias, CredentialAliasScope,
    CredentialBinding, CredentialError};
pub(crate) use boot_identity::{NativeBootIdentity, NativeBootIdentityError};
