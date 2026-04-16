pub mod codec;
pub mod delta;
pub mod dictionary;
pub mod error;
pub mod gorilla;
pub mod simple8b;

pub use codec::Codec;
pub use error::{CompressError, CompressResult};
