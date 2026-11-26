//! A correctly rounded sum of finite `f64` terms (#437): the nearest `f64` to their exact
//! real sum, whatever order they are added in and however they are split between partial
//! sums. Terms are kept as a nonoverlapping expansion (Shewchuk's grow-expansion, as
//! Python's `math.fsum`), so `add` and `merge` are exact; [`ExactSum::value`] rounds once,
//! half to even.

#[derive(Clone, Debug, Default)]
pub(crate) struct ExactSum {
    /// Nonoverlapping partials in increasing magnitude; their exact sum is the sum.
    partials: Vec<f64>,
}

impl ExactSum {
    /// Adds `x` exactly. `x` must be finite.
    pub(crate) fn add(&mut self, mut x: f64) {
        let mut kept = 0;
        for j in 0..self.partials.len() {
            let mut y = self.partials[j];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0. {
                self.partials[kept] = lo;
                kept += 1;
            }
            x = hi;
        }
        self.partials.truncate(kept);
        self.partials.push(x);
    }

    /// Adds every term of `other` exactly.
    pub(crate) fn merge(&mut self, other: &Self) {
        for &p in &other.partials {
            self.add(p);
        }
    }

    /// The correctly rounded sum (`0` for no terms).
    pub(crate) fn value(&self) -> f64 {
        let p = &self.partials;
        let Some((&last, rest)) = p.split_last() else {
            return 0.;
        };
        let mut n = rest.len();
        let (mut hi, mut lo) = (last, 0.);
        // Add from the top until the sum is inexact.
        while n > 0 {
            let x = hi;
            n -= 1;
            let y = p[n];
            hi = x + y;
            let yr = hi - x;
            lo = y - yr;
            if lo != 0. {
                break;
            }
        }
        // Half-way cases: round as the next partial (same sign as `lo`) decides.
        if n > 0 && ((lo < 0. && p[n - 1] < 0.) || (lo > 0. && p[n - 1] > 0.)) {
            let y = lo * 2.;
            let x = hi + y;
            if y == x - hi {
                hi = x;
            }
        }
        hi
    }
}

#[cfg(test)]
mod tests {
    use super::ExactSum;

    fn sum(terms: &[f64]) -> f64 {
        let mut s = ExactSum::default();
        for &t in terms {
            s.add(t);
        }
        s.value()
    }

    /// Exact where a naive sum is not, and independent of order and of splitting.
    #[test]
    fn sums_are_correctly_rounded_and_order_free() {
        assert_eq!(sum(&[]), 0.);
        assert_eq!(sum(&[1e16, 1., -1e16]), 1.);
        assert_eq!(sum(&[0.1; 10]), 1.);
        assert_ne!([0.1f64; 10].iter().sum::<f64>(), 1.);
        // Half-way: 1 + 2^-53 + 2^-53·2^-60 rounds up only because of the last term.
        let tiny = f64::EPSILON / 2.;
        assert_eq!(sum(&[1., tiny, tiny * 2f64.powi(-60)]), 1. + f64::EPSILON);
        assert_eq!(sum(&[1., tiny]), 1.);
        let terms: Vec<f64> = (0..10_000)
            .map(|i| ((i * 7919 % 10_007) as f64).sqrt() * 1e-3 + (i as f64) * 1e5)
            .collect();
        let forward = sum(&terms);
        let mut reversed = terms.clone();
        reversed.reverse();
        assert_eq!(sum(&reversed), forward);
        let (a, b) = terms.split_at(3_333);
        let (mut left, mut right) = (ExactSum::default(), ExactSum::default());
        a.iter().for_each(|&t| left.add(t));
        b.iter().for_each(|&t| right.add(t));
        right.merge(&left);
        assert_eq!(right.value(), forward);
    }
}
