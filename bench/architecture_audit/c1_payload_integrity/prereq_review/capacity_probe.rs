// Independent std-only probe of requested Vec storage, not a codec execution.
fn main() {
    for length in [2usize, 4, 6, 8, 10] {
        let backing = vec![0u32; length.div_ceil(4)];
        let mut old = Vec::<u8>::with_capacity(length);
        for word in &backing { old.extend(word.to_ne_bytes()); }
        old.truncate(length);
        let mut exact = Vec::<u8>::with_capacity(length);
        for word in &backing {
            let remaining = length - exact.len();
            exact.extend_from_slice(&word.to_ne_bytes()[..remaining.min(4)]);
        }
        assert_eq!(old, exact);
        assert_eq!(exact.capacity(), length);
        let peak = backing.capacity() * 4 + exact.capacity();
        assert!(peak <= 2 * length + 3);
        println!("length={length} backing={} old_copy={} exact_copy={} exact_peak={peak}", backing.capacity()*4, old.capacity(), exact.capacity());
    }
}
