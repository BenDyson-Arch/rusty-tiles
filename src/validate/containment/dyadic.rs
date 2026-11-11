//! Arena-backed sign-magnitude dyadic rationals `±m·2^e`, with odd `m` in little-endian u64 limbs.
//!
//! Every value lives in one `Vec<u64>` reserved once with `try_reserve_exact`; callers hold only
//! index handles. Results are exact: nothing rounds. A result that would exceed its value's limb
//! capacity is a capacity-invariant breach (the static span bound is wrong): it panics in debug
//! builds and otherwise sets the sticky fault and stores zero, never a wrapped value.
use std::cmp::Ordering;

/// Admission values; the last is operation scratch.
pub(super) const ADMIT_VALUES: usize = 4;
/// `⌈(2151 + 64)/64⌉`: `Σq² − 1` with f64 `|q_i| ≤ 1` has set bits within `[−2148, 2]`.
pub(super) const ADMIT_LIMBS: usize = 35;
/// Exponent and `len | neg << 63` precede each value's limbs.
const HEAD: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct D(usize);

pub(super) struct Dyadics {
    v: Vec<u64>,
    /// Limbs per walk value.
    cap: usize,
    /// Walk values, including the trailing operation scratch.
    n: usize,
    fault: bool,
}

#[derive(Clone, Copy)]
struct Head {
    neg: bool,
    exp: i64,
    len: usize,
    at: usize,
}

impl Dyadics {
    /// `n` walk values of `cap` limbs plus the admission region, refused above `max_limbs`.
    pub(super) fn reserve(n: usize, cap: usize, max_limbs: usize) -> Option<Self> {
        let total = n
            .checked_mul(cap.checked_add(HEAD)?)?
            .checked_add(ADMIT_VALUES * (ADMIT_LIMBS + HEAD))?;
        if n < 2 || cap == 0 || total > max_limbs {
            return None;
        }
        let mut v = Vec::new();
        v.try_reserve_exact(total).ok()?;
        v.resize(total, 0);
        Some(Self {
            v,
            cap,
            n,
            fault: false,
        })
    }
    pub(super) fn limbs(&self) -> usize {
        self.v.len()
    }
    /// Walk value `i`; the operation scratch is not addressable.
    pub(super) fn walk(&self, i: usize) -> D {
        debug_assert!(i + 1 < self.n);
        D(i)
    }
    pub(super) fn admit(&self, i: usize) -> D {
        debug_assert!(i + 1 < ADMIT_VALUES);
        D(self.n + i)
    }
    pub(super) fn fault(&self) -> bool {
        self.fault
    }
    /// Records a capacity-invariant breach.
    pub(super) fn breach(&mut self) {
        self.fault = true;
        if cfg!(debug_assertions) {
            panic!("dyadic capacity exceeded: the static span bound does not cover a value");
        }
    }
    fn slot(&self, d: D) -> (usize, usize) {
        if d.0 < self.n {
            (d.0 * (self.cap + HEAD), self.cap)
        } else {
            let base = self.n * (self.cap + HEAD);
            (base + (d.0 - self.n) * (ADMIT_LIMBS + HEAD), ADMIT_LIMBS)
        }
    }
    fn scratch(&self, d: D) -> D {
        D(if d.0 < self.n {
            self.n - 1
        } else {
            self.n + ADMIT_VALUES - 1
        })
    }
    fn head(&self, d: D) -> Head {
        let (o, _) = self.slot(d);
        let w = self.v[o + 1];
        Head {
            neg: w >> 63 == 1,
            exp: self.v[o] as i64,
            len: (w & !(1 << 63)) as usize,
            at: o + HEAD,
        }
    }
    fn set_head(&mut self, d: D, neg: bool, exp: i64, len: usize) {
        let (o, _) = self.slot(d);
        let neg = neg && len > 0;
        self.v[o] = if len == 0 { 0 } else { exp as u64 };
        self.v[o + 1] = len as u64 | (u64::from(neg) << 63);
    }
    pub(super) fn is_zero(&self, d: D) -> bool {
        self.head(d).len == 0
    }
    pub(super) fn is_negative(&self, d: D) -> bool {
        self.head(d).neg
    }
    pub(super) fn zero(&mut self, d: D) {
        self.set_head(d, false, 0, 0);
    }
    fn word(&mut self, d: D, neg: bool, m: u64, exp: i64) {
        if m == 0 {
            return self.zero(d);
        }
        let tz = m.trailing_zeros();
        let at = self.slot(d).0 + HEAD;
        self.v[at] = m >> tz;
        self.set_head(d, neg, exp + i64::from(tz), 1);
    }
    pub(super) fn int(&mut self, d: D, x: i64) {
        self.word(d, x < 0, x.unsigned_abs(), 0);
    }
    /// `2^e`.
    pub(super) fn pow2(&mut self, d: D, e: i64) {
        self.word(d, false, 1, e);
    }
    /// The exact value of a finite binary64; `false` (value zero) for a nonfinite input.
    pub(super) fn float(&mut self, d: D, x: f64) -> bool {
        if !x.is_finite() {
            self.zero(d);
            return false;
        }
        let bits = x.to_bits();
        let (e, f) = (((bits >> 52) & 0x7ff) as i64, bits & ((1 << 52) - 1));
        let (m, exp) = if e == 0 {
            (f, -1074)
        } else {
            (f | 1 << 52, e - 1075)
        };
        self.word(d, x.is_sign_negative(), m, exp);
        true
    }
    pub(super) fn copy(&mut self, dst: D, src: D) {
        if dst == src {
            return;
        }
        let h = self.head(src);
        let (o, cap) = self.slot(dst);
        if h.len > cap {
            self.zero(dst);
            return self.breach();
        }
        self.v.copy_within(h.at..h.at + h.len, o + HEAD);
        self.set_head(dst, h.neg, h.exp, h.len);
    }
    pub(super) fn neg(&mut self, d: D) {
        let h = self.head(d);
        self.set_head(d, !h.neg, h.exp, h.len);
    }
    /// `2·d`, by exponent.
    pub(super) fn dbl(&mut self, d: D) {
        let h = self.head(d);
        match h.exp.checked_add(1) {
            Some(exp) => self.set_head(d, h.neg, exp, h.len),
            None if h.len == 0 => {}
            None => {
                self.zero(d);
                self.breach();
            }
        }
    }
    fn bits(&self, h: Head) -> u64 {
        (h.len as u64 - 1) * 64 + u64::from(64 - self.v[h.at + h.len - 1].leading_zeros())
    }
    /// Limb `k` of `m·2^s`, without materializing the shift.
    fn shifted(&self, h: Head, s: u64, k: usize) -> u64 {
        let (q, r) = ((s / 64) as usize, (s % 64) as u32);
        let limb = |i: usize| {
            if i < h.len {
                self.v[h.at + i]
            } else {
                0
            }
        };
        let Some(i) = k.checked_sub(q) else {
            return 0;
        };
        let low = limb(i) << r;
        match (r, i.checked_sub(1)) {
            (0, _) | (_, None) => low,
            (_, Some(j)) => low | limb(j) >> (64 - r),
        }
    }
    pub(super) fn cmp_abs(&self, a: D, b: D) -> Ordering {
        let (ha, hb) = (self.head(a), self.head(b));
        match (ha.len, hb.len) {
            (0, 0) => return Ordering::Equal,
            (0, _) => return Ordering::Less,
            (_, 0) => return Ordering::Greater,
            _ => {}
        }
        // Top set-bit positions decide unless equal; then the aligned width is bounded by both.
        let top = |h: Head| i128::from(h.exp) + i128::from(self.bits(h));
        match top(ha).cmp(&top(hb)) {
            Ordering::Equal => {}
            order => return order,
        }
        let e = ha.exp.min(hb.exp);
        let (sa, sb) = (ha.exp.abs_diff(e), hb.exp.abs_diff(e));
        let n = (sa + self.bits(ha)).div_ceil(64) as usize;
        (0..n)
            .rev()
            .map(|k| self.shifted(ha, sa, k).cmp(&self.shifted(hb, sb, k)))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    }
    pub(super) fn cmp(&self, a: D, b: D) -> Ordering {
        let (ha, hb) = (self.head(a), self.head(b));
        let sign = |h: Head| match (h.len, h.neg) {
            (0, _) => 0,
            (_, true) => -1,
            _ => 1,
        };
        match sign(ha).cmp(&sign(hb)) {
            Ordering::Equal if ha.neg => self.cmp_abs(b, a),
            Ordering::Equal => self.cmp_abs(a, b),
            order => order,
        }
    }
    pub(super) fn add(&mut self, dst: D, a: D, b: D) {
        self.sum(dst, a, b, false);
    }
    pub(super) fn sub(&mut self, dst: D, a: D, b: D) {
        self.sum(dst, a, b, true);
    }
    fn sum(&mut self, dst: D, a: D, b: D, flip: bool) {
        let (ha, mut hb) = (self.head(a), self.head(b));
        hb.neg ^= flip;
        if hb.len == 0 {
            return self.copy(dst, a);
        }
        if ha.len == 0 {
            self.copy(dst, b);
            return self.set_head(dst, hb.neg, hb.exp, hb.len);
        }
        let (hi, lo) = if ha.exp >= hb.exp { (ha, hb) } else { (hb, ha) };
        let s = hi.exp.abs_diff(lo.exp);
        let t = self.scratch(dst);
        let (o, cap) = self.slot(t);
        let at = o + HEAD;
        // Exact limbs of the aligned operands, plus one carry limb.
        let n = match s.checked_add(self.bits(hi)) {
            Some(w) => (w.div_ceil(64) as usize).max(lo.len),
            None => usize::MAX,
        };
        if n >= cap {
            self.zero(dst);
            return self.breach();
        }
        for k in 0..=n {
            self.v[at + k] = self.shifted(hi, s, k);
        }
        let lo_limb = |v: &[u64], k: usize| if k < lo.len { v[lo.at + k] } else { 0 };
        let neg = if hi.neg == lo.neg {
            let mut carry = false;
            for k in 0..=n {
                let (x, c1) = self.v[at + k].overflowing_add(lo_limb(&self.v, k));
                let (x, c2) = x.overflowing_add(u64::from(carry));
                self.v[at + k] = x;
                carry = c1 || c2;
            }
            debug_assert!(!carry);
            hi.neg
        } else {
            let order = (0..n)
                .rev()
                .map(|k| self.v[at + k].cmp(&lo_limb(&self.v, k)))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal);
            if order == Ordering::Equal {
                return self.zero(dst);
            }
            let big_t = order == Ordering::Greater;
            let mut borrow = false;
            for k in 0..n {
                let (x, y) = (self.v[at + k], lo_limb(&self.v, k));
                let (x, y) = if big_t { (x, y) } else { (y, x) };
                let (r, b1) = x.overflowing_sub(y);
                let (r, b2) = r.overflowing_sub(u64::from(borrow));
                self.v[at + k] = r;
                borrow = b1 || b2;
            }
            debug_assert!(!borrow);
            if big_t {
                hi.neg
            } else {
                lo.neg
            }
        };
        self.normalize(t, neg, lo.exp, n + 1);
        self.copy(dst, t);
    }
    pub(super) fn mul(&mut self, dst: D, a: D, b: D) {
        let (ha, hb) = (self.head(a), self.head(b));
        if ha.len == 0 || hb.len == 0 {
            return self.zero(dst);
        }
        let t = self.scratch(dst);
        let (o, cap) = self.slot(t);
        let at = o + HEAD;
        let (n, exp) = (ha.len + hb.len, ha.exp.checked_add(hb.exp));
        let Some(exp) = exp.filter(|_| n <= cap) else {
            self.zero(dst);
            return self.breach();
        };
        self.v[at..at + n].fill(0);
        for i in 0..ha.len {
            let x = u128::from(self.v[ha.at + i]);
            let mut carry = 0u128;
            for j in 0..hb.len {
                let p = x * u128::from(self.v[hb.at + j]) + u128::from(self.v[at + i + j]) + carry;
                self.v[at + i + j] = p as u64;
                carry = p >> 64;
            }
            self.v[at + i + hb.len] = carry as u64;
        }
        self.normalize(t, ha.neg != hb.neg, exp, n);
        self.copy(dst, t);
    }
    /// Strips zero limbs and trailing zero bits so `m` is odd and its top limb nonzero.
    fn normalize(&mut self, t: D, neg: bool, exp: i64, len: usize) {
        let at = self.slot(t).0 + HEAD;
        let mut len = len;
        while len > 0 && self.v[at + len - 1] == 0 {
            len -= 1;
        }
        if len == 0 {
            return self.zero(t);
        }
        let z = (0..len).take_while(|&k| self.v[at + k] == 0).count();
        let tz = self.v[at + z].trailing_zeros();
        let shift = i64::try_from(z * 64)
            .ok()
            .and_then(|s| s.checked_add(i64::from(tz)))
            .and_then(|s| exp.checked_add(s));
        let Some(exp) = shift else {
            self.zero(t);
            return self.breach();
        };
        len -= z;
        for k in 0..len {
            let high = if tz > 0 && k + 1 < len {
                self.v[at + z + k + 1] << (64 - tz)
            } else {
                0
            };
            self.v[at + k] = (self.v[at + z + k] >> tz) | high;
        }
        if self.v[at + len - 1] == 0 {
            len -= 1;
        }
        self.set_head(t, neg, exp, len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena(cap: usize) -> Dyadics {
        Dyadics::reserve(8, cap, usize::MAX).unwrap()
    }
    /// Exact `i128` value of `d`, scaled by `2^-scale`.
    fn value(x: &Dyadics, d: D, scale: i64) -> i128 {
        let h = x.head(d);
        let mut m = 0i128;
        for k in (0..h.len).rev() {
            m = (m << 64) | i128::from(x.v[h.at + k]);
        }
        let e = h.exp - scale;
        assert!(h.len == 0 || e >= 0);
        let m = if h.len == 0 { 0 } else { m << e };
        if h.neg {
            -m
        } else {
            m
        }
    }
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn small_operations_match_i128() {
        let mut x = arena(4);
        let (a, b, c) = (x.walk(0), x.walk(1), x.walk(2));
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..20_000 {
            let (ma, mb) = ((rng.next() >> 34) as i64, (rng.next() >> 34) as i64);
            let (ma, mb) = (ma - (1 << 29), mb - (1 << 29));
            let (ea, eb) = ((rng.next() % 21) as i64 - 10, (rng.next() % 21) as i64 - 10);
            let set = |x: &mut Dyadics, d: D, m: i64, e: i64| {
                x.int(d, m);
                let h = x.head(d);
                x.set_head(d, h.neg, h.exp + e, h.len);
            };
            set(&mut x, a, ma, ea);
            set(&mut x, b, mb, eb);
            let (va, vb) = (i128::from(ma) << (ea + 10), i128::from(mb) << (eb + 10));
            x.add(c, a, b);
            assert_eq!(value(&x, c, -10), va + vb);
            x.sub(c, a, b);
            assert_eq!(value(&x, c, -10), va - vb);
            x.mul(c, a, b);
            assert_eq!(value(&x, c, -20), va * vb);
            assert_eq!(x.cmp(a, b), va.cmp(&vb));
            assert_eq!(x.cmp_abs(a, b), va.abs().cmp(&vb.abs()));
            // Aliased destinations.
            x.copy(c, a);
            x.add(c, c, c);
            assert_eq!(value(&x, c, -10), 2 * va);
            x.copy(c, b);
            x.mul(c, c, c);
            assert_eq!(value(&x, c, -20), vb * vb);
        }
        assert!(!x.fault());
    }

    #[test]
    fn multi_limb_ring_identities_hold() {
        let mut x = Dyadics::reserve(12, 16, usize::MAX).unwrap();
        let [a, b, c, l, r, t] = [0, 1, 2, 3, 4, 5].map(|i| x.walk(i));
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let mut random = |x: &mut Dyadics, d: D| {
            let parts = 1 + rng.next() % 3;
            x.zero(d);
            for _ in 0..parts {
                let e = (rng.next() % 400) as i64 - 200;
                x.int(t, (rng.next() >> 2) as i64 - (1 << 61));
                let h = x.head(t);
                x.set_head(t, h.neg, h.exp + e, h.len);
                x.add(d, d, t);
            }
        };
        for _ in 0..2_000 {
            random(&mut x, a);
            random(&mut x, b);
            random(&mut x, c);
            // (a + b)·c = a·c + b·c
            x.add(l, a, b);
            x.mul(l, l, c);
            x.mul(r, a, c);
            x.mul(t, b, c);
            x.add(r, r, t);
            assert_eq!(x.cmp(l, r), Ordering::Equal);
            // (a − b) + b = a, and a − b < a iff b > 0
            x.sub(l, a, b);
            let zero = x.walk(6);
            assert_eq!(x.cmp(l, a), x.cmp(zero, b));
            x.add(l, l, b);
            assert_eq!(x.cmp(l, a), Ordering::Equal);
            // |a·b| ordering agrees with signed comparison of squares.
            x.mul(l, a, a);
            x.mul(r, b, b);
            assert_eq!(x.cmp(l, r), x.cmp_abs(a, b));
        }
        assert!(!x.fault());
    }

    #[test]
    fn wide_values_cancel_exactly() {
        let mut x = arena(4);
        let (a, b, c) = (x.walk(0), x.walk(1), x.walk(2));
        x.pow2(a, 150);
        x.pow2(b, -40);
        x.add(c, a, b);
        assert_eq!(x.head(c).len, 3);
        x.sub(c, c, a);
        assert_eq!(x.cmp(c, b), Ordering::Equal);
        x.neg(b);
        x.add(c, a, b);
        x.sub(c, c, a);
        assert!(x.is_negative(c));
        assert_eq!(x.cmp_abs(c, b), Ordering::Equal);
        // (2^64 − 1)² needs two limbs and stays odd.
        x.int(a, -1);
        x.float(b, 18446744073709551615.0);
        assert_eq!(x.head(b).exp, 64);
        x.int(a, i64::MAX);
        x.mul(c, a, a);
        assert_eq!(value(&x, c, 0), i128::from(i64::MAX) * i128::from(i64::MAX));
        x.sub(c, a, a);
        assert!(x.is_zero(c));
        assert!(!x.fault());
    }

    #[test]
    fn floats_convert_exactly() {
        let mut x = arena(2);
        let d = x.walk(0);
        for v in [
            1.5,
            -3.0,
            0.25,
            1000.25,
            f64::MAX,
            f64::MIN_POSITIVE,
            5e-324,
            -0.0,
            0.0,
        ] {
            assert!(x.float(d, v));
            let h = x.head(d);
            if v == 0.0 {
                assert_eq!(h.len, 0);
                assert!(!x.is_negative(d));
                continue;
            }
            let m = x.v[h.at];
            assert_eq!(m % 2, 1);
            assert_eq!(h.neg, v < 0.0);
            // m·2^e reproduces v exactly through powi of exact powers of two.
            let back = m as f64
                * 2f64.powi(h.exp.clamp(-1022, 1023) as i32)
                * 2f64.powi((h.exp - h.exp.clamp(-1022, 1023)) as i32);
            assert_eq!(back, v.abs());
        }
        assert!(!x.float(d, f64::NAN));
        assert!(!x.float(d, f64::INFINITY));
        assert!(x.is_zero(d));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "capacity exceeded"))]
    fn capacity_breach_is_sticky_not_wrapped() {
        let mut x = arena(1);
        let (a, b) = (x.walk(0), x.walk(1));
        x.int(a, (1 << 62) + 1);
        x.mul(b, a, a);
        assert!(x.fault());
        assert!(x.is_zero(b));
        x.int(b, 3);
        assert!(x.fault());
    }

    #[test]
    fn admission_region_is_separate_and_wide() {
        let mut x = arena(1);
        let (s, t) = (x.admit(0), x.admit(1));
        x.pow2(s, 2);
        x.float(t, 5e-324);
        x.mul(t, t, t);
        x.add(s, s, t);
        assert_eq!(x.head(s).len, 34);
        x.sub(s, s, t);
        assert_eq!(value(&x, s, 0), 4);
        assert!(!x.fault());
        assert_eq!(x.limbs(), 8 * 3 + ADMIT_VALUES * (ADMIT_LIMBS + HEAD));
    }

    #[test]
    fn reservation_refuses_without_panicking() {
        assert!(Dyadics::reserve(8, 2, 10).is_none());
        assert!(Dyadics::reserve(usize::MAX / 2, 4, usize::MAX).is_none());
        assert!(Dyadics::reserve(1 << 40, 1 << 20, usize::MAX).is_none());
    }
}
