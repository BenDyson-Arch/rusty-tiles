//! Owned local glTF admission, metadata and decoding boundaries shared by mesh consumers.
//! Consumers own input bytes or streaming file bindings; this module does not invoke writers.
pub(crate) mod binding;
pub(crate) mod source;
