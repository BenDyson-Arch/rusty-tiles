//! Private, bounded format meaning. Callers own paths, policy and causal I/O.
pub(crate) mod json;
pub(crate) mod meshopt;
pub(crate) mod numbers;
pub(crate) mod payload;

#[derive(Clone, Copy, Debug)]
pub(crate) struct JsonLimits {
    pub bytes: usize,
    pub depth: u64,
    pub value_nodes: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PayloadLimits {
    pub member_bytes: usize,
    pub decoded_bytes: usize,
    pub accessor_components: u64,
    pub json: JsonLimits,
}

#[derive(Debug)]
pub(crate) enum FormatError {
    InvalidInput(String),
    Unsupported(String),
    ResourceLimit(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message)
            | Self::Unsupported(message)
            | Self::ResourceLimit(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for FormatError {}

#[derive(Debug)]
pub(crate) enum PayloadError<E> {
    Format(FormatError),
    Resolver(E),
}
impl<E> From<FormatError> for PayloadError<E> {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}
