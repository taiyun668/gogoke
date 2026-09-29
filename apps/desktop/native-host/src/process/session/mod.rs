//! Design 37 native seat process boundary. No Node frame can select a token.

mod isolation;

pub(crate) use isolation::{AppContainerProfile, SecurityCapabilities};
