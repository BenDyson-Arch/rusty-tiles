//! Leaf encoding foundations. The producer owns their frozen plans and budgets.
pub(super) mod atlas;
mod crops;
mod delivery;
pub(super) mod geometry;
#[allow(dead_code)] // Requested by H13 baking and the future annotate pass.
pub(crate) mod id_texture;
mod mask;
mod mips;
pub(super) mod planning;
mod prepared;
pub(crate) mod producer;
mod schedule;
mod sparse_mips;
mod writer;

#[cfg(test)]
mod calibration_tests;
#[cfg(test)]
mod mip_oracle_tests;

mod source_sampler_mips;
