use crate::content_integrity::{FormatError, JsonLimits};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressionLimits {
    pub source_bytes: usize,
    pub json_bytes: usize,
    pub json_depth: u64,
    pub json_value_nodes: usize,
    pub buffer_views: usize,
    pub accessors: usize,
    pub logical_bytes: usize,
    pub candidate_bytes: usize,
    pub working_bytes: usize,
}
impl Default for CompressionLimits {
    fn default() -> Self {
        Self {
            source_bytes: 33_554_432,
            json_bytes: 1_048_576,
            json_depth: 64,
            json_value_nodes: 65_536,
            buffer_views: 4096,
            accessors: 4096,
            logical_bytes: 33_554_432,
            candidate_bytes: 67_108_864,
            working_bytes: 134_217_728,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedCompressionLimits {
    pub(crate) values: CompressionLimits,
}
impl CompressionLimits {
    pub(crate) fn validate(self) -> Result<ValidatedCompressionLimits, FormatError> {
        if [
            self.source_bytes,
            self.json_bytes,
            self.json_value_nodes,
            self.buffer_views,
            self.accessors,
            self.logical_bytes,
            self.candidate_bytes,
            self.working_bytes,
        ]
        .contains(&0)
        {
            return Err(FormatError::InvalidInput(
                "compression limits must be positive".into(),
            ));
        }
        if self.source_bytes > u32::MAX as usize
            || self.candidate_bytes > u32::MAX as usize
            || [
                self.source_bytes,
                self.json_bytes,
                self.logical_bytes,
                self.candidate_bytes,
                self.working_bytes,
            ]
            .iter()
            .any(|n| *n > isize::MAX as usize)
        {
            return Err(FormatError::InvalidInput(
                "compression limit outside host/GLB domain".into(),
            ));
        }
        Ok(ValidatedCompressionLimits { values: self })
    }
}
impl ValidatedCompressionLimits {
    pub(super) fn json(self) -> JsonLimits {
        JsonLimits {
            bytes: self.values.json_bytes,
            depth: self.values.json_depth,
            value_nodes: self.values.json_value_nodes,
        }
    }
}
