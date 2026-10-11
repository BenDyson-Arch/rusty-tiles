//! Arena-backed sign-magnitude dyadic integers: `±m·2^e` with `m` in u64 limbs.

pub(super) struct Dyadics {
    limbs: Vec<u64>,
}
