//! Design 37 native seat process boundary. No Node frame can select a token.

mod isolation;
mod compat_module;

pub(crate) use isolation::{AppContainerProfile, IsolationError, SecurityCapabilities};
pub(crate) use compat_module::{CompatModule, DirectoryRoots};
