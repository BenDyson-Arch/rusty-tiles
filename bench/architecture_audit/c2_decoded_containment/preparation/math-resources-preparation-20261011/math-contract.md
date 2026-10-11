# C2 represented frame and containment choices — preparation

This is author preparation, not implementation, final-source acceptance or a
release claim. C2 operates on finite static decoded draw-position instances in
the domain author's explicit local GLB/glTF/modern-b3dm profile. C1 remains a
distinct integrity inspection; unsupported C2 scene meaning does not retroactively
make a C1 payload malformed.

## Meaning and normative boundaries

Retained primary sources are byte-exact copies of the domain author's pinned
Khronos glTF2.0.1, 3D Tiles core and modern-b3dm bodies. See input-pin.json and
primary/primary-pins.json for identities/acquisition. Published OGC22-025r4
[tile transforms and volumes](https://docs.ogc.org/cs/22-025r4/22-025r4.html)
were located with web search; the full52,897,656-byte HTML body could not be opened
by the web tool and is not retained here. The retained pinned Tiles source is
the actual inspectable body. This is an explicit primary-source limitation.

Tiles transforms are column-major local-to-parent affine transforms. glTF uses
parent*local, TRS T*R*S and y-up; runtime U(x,y,z)=(x,-z,y) precedes b3dm
RTC_CENTER translation and tile transforms. Regions use geographic coordinates
and are exempt from tile transform application. These establish order and frame
meaning, not a numerical tolerance. The primary core spatial-coherence condition
concerns child CONTENT inside parent volumes; containment of every declared child
VOLUME is a separate stronger check. C2 must check every content point in every
applicable ancestor even if declared child bounds are generous or cached.

The numerical statement is about represented input scalars, not ideal source
measurements or arbitrary original JSON decimal reals. A JSON numeric scalar is
the selected finite binary64 value after the consuming numeric adapter. Document
this representation adapter; valid JSON outside finite representation is
Unsupported, not malformed number grammar. A finite f32 POSITION word promoted
to f64 and signed/unsigned <=32-bit integers are exact. Normalized i8/u8/i16/u16
POSITION is an exact rational clamp/division by127/255/32767/65535. Supply raw
integer plus normalization tag to the math owner, who encloses that rational;
the rounded quotient must not silently become exact geometric truth. Declared
accessor min/max are integrity facts, never a replacement for draw positions.
The domain author's modern-b3dm RTC proposal is a specific semantic adapter:
finite JSON binary64 is converted once to nearest/ties-even f32, refusing an
overflowing result, then promoted exactly to f64; binary RTC is already exact
little-endian f32. Its primary table declares float32[3]. Record this explicit
conversion rather than treating every JSON table decimal as a general binary64
RTC coordinate. The whole gate must approve this source datatype interpretation.

## Small private representation

Use fixed-size finite Cartesian scalars, an affine3x4 enclosure and a3-component
point enclosure. A local transform is Matrix16 or exact TRS facts, supplied by
the format consumer. A scalar interval stores finite lo<=hi endpoints. A verdict
is Contained, Outside or NumericalUnresolved. Keep numerical cause finite
(NonfiniteArithmetic, AmbiguousBoundary, IndeterminateBasis) and map it once to
C2 Unsupported. A proved outside point is InvalidInput. Work/capacity refusal
is ResourceLimit. None of these becomes a skip, a successful C1-only fallback,
or an inflated volume.

The minimal private callback discussed with the domain owner is NodeEnter
(index,local Matrix/TRS), DrawPosition (mesh/primitive/draw-index,raw3,normalization)
and NodeExit. Existing C1 views/accessors remain live while the callback runs.
The callback carries no path policy, public scene IR or source-sized strings;
consumer/resolver causes remain separate from format errors. No full POSITION
Vec or second decoder. The actual authored callback/drop/error source ledger is
a later implementation-specific obligation.

Forward local node frames may be singular: glTF zero scales remain legitimate
and positions are transformed forward. A cumulative matrix may be sheared by
valid nested nonuniform scales/rotations; do not impose a local no-shear rule on
it. Tile local transforms are affine, with separate C2 nonsingularity admission:
certify each3x3 determinant interval excludes0; exact zero or unresolved sign is
C2 Unsupported, not a claim that the Tiles affine transform is malformed.

After node transforms, apply exact signed permutation U and then RTC. To test
the tile's own/content bounds, use this tile-local point. To test its parent and
further ancestors, apply successive child-to-parent tile transforms including
external tileset roots. Each suffix is forward-only. Nonsingular tile profile
makes local membership equivalent under the common outer frame; it avoids
arbitrary matrix inversion and subtracting huge world/ECEF translations.
Keep each content reference and core mesh instance distinct. A shared URI or
mesh never licenses reusing a frame verdict.

## Outward arithmetic and exact shortcuts

Assume the actual supported target uses IEEE binary64 nearest/ties-even ordinary
operations, with gradual underflow and no fast-math reassociation. The first
actual source/compiler review must verify its operations and sensitive controls.
For addition/subtraction/multiplication, use interval endpoint formulas and
next-down/next-up around finite rounded results. An operation yielding NaN/inf,
or a nonfinite outward endpoint, returns NumericalUnresolved. Division requires
a denominator interval excluding0 and uses outward endpoint quotients. A
normalized nontrivial integer ratio is initialized by the adjacent outward
endpoints of its rounded quotient. Signed minimum clamps to exact-1.

Maintain dependency-safe exact singleton shortcuts: identity/zero/negation,
subtraction of equal singleton scalars, adding exact integer singletons of
magnitude<=2^52 (sum magnitude<=2^53), and multiplying exact integer singletons
of magnitude<=2^26 (product magnitude<=2^52). These give useful exact closed
boundary cases without a bignum/FMA/heuristic repair framework. The bounded
integer magnitude conditions are part of the proof; arbitrary rounded
product-equality checks are not exactness certificates. An interval equal to
another non-singleton interval cannot be cancelled as though its unknown values
were the same.

Compose affine enclosures with fixed operation order and check every intermediate
owner/error. No tolerance is added for large coordinate magnitude. Width reflects
arithmetic uncertainty only. A broad interval does not become a broad accepted
geometry domain. Exact zero/identity short circuits avoid unnecessary widening;
other closed-boundary cases may explicitly remain unresolved.

## Cartesian predicates

Sphere center c/radius r>=0: form outward d=p-c, interval sum(d_i^2) and interval
r^2. Square bounds use min0 when an input crosses0. Contained requires
upper(sum)<=lower(r^2); Outside requires lower(sum)>upper(r^2). Otherwise
NumericalUnresolved. Radius0 admits an exact point at its center. Squared-distance
overflow/underflow cannot cause an acceptance; no sqrt/libm error promise and
no scale_bound radius inflation are needed for finite point instances.

Box represented-set proposal: c+B[-1,1]^3, with supplied half-axis columns B.
For a nonsingular finite B form fixed cross products C_i and det(B), all outward.
Require determinant interval excludes0. Cramer's rule says |dot(p-c,C_i)|<=|det|
for all three coordinates. Use absolute intervals and compare numerator upper
to determinant lower for Contained; any numerator lower>determinant upper proves
Outside. Otherwise unresolved. This uses no division/inverse approximation or
condition-number threshold; determinant separation is the numerical certificate.

For an exact diagonal or signed-permuted axis-aligned basis, compare each
coordinate difference to its exact half-length directly. Zero half-lengths
define closed planes/lines/points and require exact equality to succeed. Other
rank-deficient/rotated-degenerate boxes are initially C2 Unsupported; a general
rank solver is deferred. General represented nonsingular basis membership also
handles tiny nonorthogonality without normalizing directions. The primary calls
this an oriented box and describes matrix columns as images of unit vectors;
the combined gate must explicitly settle this represented-set interpretation
versus an exact-orthogonality-only profile. The old1e-8/1e-12 orthogonality
heuristic is not independent normative authority.

Region decoded containment is initially C2 Unsupported. Do not convert ECEF
positions to geography through the current arbitrary inverse, reuse modular
region-volume checks as a point verdict, or invent a libm/geodesy bound. C1's
existing declared region check can remain separately named historical meaning.
Neither mixed Cartesian/region nor region-only C2 receives certification.

## Proposed local glTF encoding admission

The domain owner's proposed quaternion window is components in[-1,1] with exact
represented norm^2 in1±2^-20; zero/dimension/type/out-of-component-range are
InvalidInput and outside/unresolved window is C2 Unsupported. Use supplied
components in the standard polynomial matrix formula, no renormalization.
For an exact unit quaternion rounded componentwise to f32, |e_i|<=2^-24 and
||q||1<=2 imply norm² perturbation<=2^-22+2^-46<2^-21. Thus the proposed window
is a sufficient broad encoding policy. It is NOT proof of exact unit norm.

Likewise proposed local supplied-matrix column squared-dot<=2^-40 times squared
norm product is a finite approximate TRS admission policy, not exact no-shear
decomposition proof. Evaluate it with certified enclosures or exact bounded
numeric reference; unresolved is Unsupported. Allow zero columns. Neither
window repairs the supplied transform or adds spatial containment slack. If
the product needs strict normative glTF local-transform validity rather than
this explicit represented encoding profile, that is a concrete combined-gate
decision, not something the interval containment primitive silently establishes.

## Independent preparation evidence and later checks

model.py is a tiny standalone Python interval model with an independent Fraction
reference. Sphere truth uses exact rational squared distance; box truth uses
rational Gaussian elimination, distinct from interval Cramer products.16 literal
cases include exact/one-ULP boundaries, rotated basis, axis-aligned flat boxes,
normalization, large relative-slack false acceptance, affine cancellation and
overflow/underflow.5 sensitive controls execute modeled historical slack or
rounded/underflow arithmetic and show the wrong acceptance against exact truth.
Actual model scheduler is NI10, affinity[0,1]. This is not executed Rust/native
or allocator evidence. The first model expected Unresolved where the interval
already proved Outside; preserve model-first.py and model-first-failure.json.
The successful earlier epoch is preserved in model-second.py/observed-second.json.

Later authored tests must additionally cover actual decoded streams, nested
node/tile/external transforms, U/RTC wrong-order mutants, repeated-URI different
frames, indexed unused outlier/restart positions, direct zero-scale instances,
local validity policy boundaries, malformed event graph, cancellation/cause
transport and real capacity/visit limits. Full source/actual nonauthor acceptance
must precede any default claim or merge. These obligations are concrete future
implementation evidence, not demands to prove absent code now.
