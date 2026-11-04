//! Independent test arithmetic: every finite f32 is an integer times 2^-149.
//! Evaluate |d|²(u.w) - (u.d)(w.d) with signed integer limbs, rather than the
//! producer's cross-product expansions. Its sign is unaffected by that common
//! positive scale. Coordinates use at most 277 bits; differences use at most 278,
//! dot-product magnitudes are <2^558 and the final magnitude is <2^1117.
use super::{Edge, Point};
use std::cmp::Ordering;

/// Little-endian base-2^32 sign/magnitude integer, with no high zero limbs and
/// positive zero. Only addition, subtraction and multiplication are needed here.
#[derive(Default)]
struct Integer {
    negative: bool,
    limbs: Vec<u32>,
}
impl Integer {
    fn normalized(negative: bool, mut limbs: Vec<u32>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self {
            negative: negative && !limbs.is_empty(),
            limbs,
        }
    }
    fn coordinate(bits: u32) -> Self {
        let exponent = (bits >> 23) & 255;
        assert_ne!(exponent, 255, "fold census requires finite f32 coordinates");
        let mantissa = (bits & 0x7fffff) | if exponent == 0 { 0 } else { 1 << 23 };
        let shift = exponent.saturating_sub(1) as usize;
        let value = u64::from(mantissa) << (shift % 32);
        let mut limbs = vec![0; shift / 32 + 2];
        limbs[shift / 32] = value as u32;
        limbs[shift / 32 + 1] = (value >> 32) as u32;
        Self::normalized(bits >> 31 != 0, limbs)
    }
    fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            let mut limbs = Vec::new();
            let mut carry = 0u64;
            for i in 0..self.limbs.len().max(other.limbs.len()) {
                let sum = u64::from(*self.limbs.get(i).unwrap_or(&0))
                    + u64::from(*other.limbs.get(i).unwrap_or(&0))
                    + carry;
                limbs.push(sum as u32);
                carry = sum >> 32;
            }
            limbs.push(carry as u32);
            return Self::normalized(self.negative, limbs);
        }
        let order = self
            .limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()));
        let (large, small) = if order == Ordering::Less {
            (other, self)
        } else {
            (self, other)
        };
        let mut limbs = Vec::new();
        let mut borrow = 0i64;
        for (i, &a) in large.limbs.iter().enumerate() {
            let difference = i64::from(a) - i64::from(*small.limbs.get(i).unwrap_or(&0)) - borrow;
            limbs.push(difference as u32);
            borrow = i64::from(difference < 0);
        }
        debug_assert_eq!(borrow, 0);
        Self::normalized(large.negative, limbs)
    }
    fn sub(&self, other: &Self) -> Self {
        self.add(&Self {
            negative: !other.negative,
            limbs: other.limbs.clone(),
        })
    }
    fn mul(&self, other: &Self) -> Self {
        let mut limbs = vec![0; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.limbs.iter().enumerate() {
                // The product plus an existing limb and carry fits in u64.
                let product = u64::from(a) * u64::from(b) + u64::from(limbs[i + j]) + carry;
                limbs[i + j] = product as u32;
                carry = product >> 32;
            }
            if !other.limbs.is_empty() {
                limbs[i + other.limbs.len()] = carry as u32;
            }
        }
        Self::normalized(self.negative != other.negative, limbs)
    }
    fn sign(&self) -> Ordering {
        if self.limbs.is_empty() {
            Ordering::Equal
        } else if self.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

pub(super) fn wedge_sign(edge: Edge, c: Point, e: Point) -> Ordering {
    let difference = |p: Point| -> [Integer; 3] {
        std::array::from_fn(|i| Integer::coordinate(p[i]).sub(&Integer::coordinate(edge.0[i])))
    };
    let dot = |p: &[Integer; 3], q: &[Integer; 3]| {
        (0..3).fold(Integer::default(), |sum, i| sum.add(&p[i].mul(&q[i])))
    };
    let (d, u, w) = (difference(edge.1), difference(c), difference(e));
    dot(&d, &d)
        .mul(&dot(&u, &w))
        .sub(&dot(&u, &d).mul(&dot(&w, &d)))
        .sign()
}
