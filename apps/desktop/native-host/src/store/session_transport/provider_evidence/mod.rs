//! Pure vendor data extracted only after native source capture.
//! These values never supply caller authority, delivery proof, or StopFact.
pub(crate) mod acp;
pub(crate) mod stream_json;
pub(crate) mod claude_question;
pub(crate) mod commands;
pub(crate) mod normalize;
