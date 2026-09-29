//! Native-only Design 37 request framing. The existing Node service pipe stays unwired.
mod wire;

pub(crate) use wire::{decode_request, encode_receipt, V37Request, V37Status, V37WireError};
