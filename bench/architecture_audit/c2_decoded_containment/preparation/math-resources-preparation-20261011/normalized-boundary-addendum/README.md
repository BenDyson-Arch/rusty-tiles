# Exact normalized endpoint addendum

Preserve the original C2 math/resource preparation freeze unchanged. This additive
standalone model initializes raw0 as exact0, n==denominator as exact+1,
n==-denominator and signed saturation n<=-denominator as exact-1 before any
nontrivial interval division. It adds four closed-unit-box cases for255/255,
65535/65535, signed127 and32767 saturation/plus-one/zero.

These are exact integer/rational identities, not a rounded quotient comparison.
For signed normalization the glTF clamp is applied before truth/interval output.
Other nontrivial quotients still receive outward bounds. General exactly
representable noninteger binary operations, e.g1.5*0.5, are mathematically exact
but the original shortcut model may conservatively widen them. This model does
not claim a complete exactness detector; later production may use additional
independently proved no-error operations or explicitly refuse ambiguous cases.
No production/decoder/native/allocator/default acceptance follows.
