//! Leaf encoding foundations. The producer owns their frozen plans and budgets.
pub(super) mod atlas;
pub(in crate::mesh_large) mod crops;
pub(in crate::mesh_large) mod delivery;
pub(in crate::mesh_large) mod geometry;
#[allow(dead_code)] // Requested by H13 baking and the future annotate pass.
pub(crate) mod id_texture;
pub(in crate::mesh_large) mod mask;
pub(in crate::mesh_large) mod mips;
pub(super) mod planning;
pub(in crate::mesh_large) mod prepared;
pub(crate) mod producer;
mod schedule;
pub(in crate::mesh_large) mod sparse_mips;
mod writer;

#[cfg(test)]
mod calibration_tests;
#[cfg(test)]
mod mip_oracle_tests;

mod source_sampler_mips;
