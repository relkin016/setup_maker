//! Setup Maker core: project model, payload format, codecs, condition
//! expressions and built-in installer messages.

pub mod codec;
pub mod expr;
pub mod format;
pub mod messages;
pub mod project;
pub mod wildcard;

/// Version of the whole tool chain (builder + stub must match).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
