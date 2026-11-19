// Interior-complete two-sided distance certificate. Approximate geometry chooses a
// witness only. Its three snapped simplex coordinates and residuals bound the
// entire closed panel, including degenerate triangles, by convex interpolation.
#include <algorithm>
#include <array>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <memory>
#include <new>
#include <numeric>
#include <vector>

namespace {
using V = std::array<double, 3>;
using Face = std::array<V, 3>;
using U = uint32_t;
using Raw = float[3][3];
// The callback is synchronous. A nonzero return stops the job before any result
// is used. Face counts cover completed panels' domain triangles, both directions.
using Callback = int (*)(void *, uint32_t);
struct Interrupted {};
struct InvalidArithmetic {};
void check_arithmetic() {
    // The observer may change its thread's floating-point environment.
    volatile float smallest = std::numeric_limits<float>::denorm_min();
    if (std::fegetround() != FE_TONEAREST || static_cast<double>(smallest) != 0x1p-149)
        throw InvalidArithmetic{};
}
struct Progress {
    Callback callback;
    void *context;
    U pending = 0;
    U ticks = 0;
    void flush() {
        if (callback(context, pending))
            throw Interrupted{};
        check_arithmetic();
        pending = 0;
        ticks = 0;
    }
    void tick() {
        if (++ticks >= 256)
            flush();
    }
};
constexpr int MaxDepth = 7;
constexpr U Denominator = 1u << 22;

V sub(V a, V b) {
    for (int k = 0; k < 3; ++k)
        a[k] -= b[k];
    return a;
}
V add(V a, V b) {
    for (int k = 0; k < 3; ++k)
        a[k] += b[k];
    return a;
}
V scale(V a, double s) {
    for (auto &x : a)
        x *= s;
    return a;
}
double dot(V a, V b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
V cross(V a, V b) {
    return {a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]};
}
V center(const Face &f) { return scale(add(add(f[0], f[1]), f[2]), 1. / 3.); }
V mid(V a, V b) { return scale(add(a, b), .5); }
Face lift(const Raw &f) {
    Face r;
    for (int i = 0; i < 3; ++i)
        for (int k = 0; k < 3; ++k)
            r[i][k] = f[i][k];
    return r;
}
struct Choice {
    double u, v, dsq;
};
struct Plane {
    Face f;
    V e, g, n, du, dv, bc;
    double inverse_normal2, ie, ig, ib;
    explicit Plane(Face t)
        : f(t), e(sub(f[1], f[0])), g(sub(f[2], f[0])), n(cross(e, g)), bc(sub(f[2], f[1])) {
        auto inv = [](double d) { return d > 0 ? 1. / d : 0.; };
        inverse_normal2 = inv(dot(n, n));
        du = scale(cross(g, n), inverse_normal2);
        dv = scale(cross(n, e), inverse_normal2);
        ie = inv(dot(e, e));
        ig = inv(dot(g, g));
        ib = inv(dot(bc, bc));
    }
    Choice choose(V p) const {
        V t = sub(p, f[0]);
        double u = dot(t, du), v = dot(t, dv);
        if (inverse_normal2 > 0 && u >= 0 && v >= 0 && u + v <= 1) {
            double h = dot(t, n);
            return {u, v, h * h * inverse_normal2};
        }
        Choice best{0, 0, dot(t, t)};
        auto edge = [&](V a, V dir, double inv, int k) {
            double l = std::clamp(dot(a, dir) * inv, 0., 1.);
            V d = sub(a, scale(dir, l));
            double dsq = dot(d, d);
            if (dsq < best.dsq)
                best = {k == 0 ? l : k == 1 ? 1 - l : 0., k == 1 ? l : k == 2 ? l : 0., dsq};
        };
        if (!(inverse_normal2 > 0) || v < 0.)
            edge(t, e, ie, 0);
        if (!(inverse_normal2 > 0) || u + v > 1.)
            edge(sub(p, f[1]), bc, ib, 1);
        if (!(inverse_normal2 > 0) || u < 0.)
            edge(t, g, ig, 2);
        return best;
    }
};
// Each direction owns one Plane, center, order and at most one Node per
// target face. A separate u32 seed belongs to each domain face. The Rust
// admission includes both Face arrays, recursion stacks and vector headers.
struct Node {
    V lo, hi;
    U first, end, left = 0, right = 0;
};
// One target index per direction. Each face has at most one BVH node, a
// Morton key, three edge links and four per-range visit stamps. During edge
// assembly the open-address table has fewer than ten 32-bit slots per face.
static_assert(sizeof(Plane) + sizeof(V) + sizeof(U) + sizeof(Node) + sizeof(Raw) +
                  sizeof(uint64_t) + (3 + 4 + 10) * sizeof(U) <=
              512);
struct Witness {
    U id;
    double dsq;
};
struct Patch {
    U id;
    std::array<Choice, 3> weights;
    double price;
};
// For original binary32 corners every cross-product term in the six-term
// determinant is exact in binary64. Use a floating expansion for ambiguous signs.
static int turn(const V &a, const V &b, const V &c, int x, int y) {
    const double u = (b[x] - a[x]) * (c[y] - a[y]), v = (b[y] - a[y]) * (c[x] - a[x]);
    const double d = u - v, tol = 0x1p-48 * (std::abs(u) + std::abs(v));
    if (d > tol)
        return 1;
    if (d < -tol)
        return -1;
    if ((a[x] == b[x] && b[x] == c[x]) || (a[y] == b[y] && b[y] == c[y]) ||
        (a[x] == c[x] && a[y] == c[y]) || (b[x] == c[x] && b[y] == c[y]))
        return 0;
    double e[7] = {};
    unsigned n = 0;
    for (double h :
         {a[x] * b[y], b[x] * c[y], c[x] * a[y], -a[x] * c[y], -b[x] * a[y], -c[x] * b[y]}) {
        unsigned m = 0;
        for (unsigned i = 0; i < n; ++i) {
            const double z = h + e[i], bpart = z - h;
            const double err = (h - (z - bpart)) + (e[i] - bpart);
            if (err != 0.)
                e[m++] = err;
            h = z;
        }
        if (h != 0.)
            e[m++] = h;
        n = m;
    }
    return n ? (e[n - 1] > 0 ? 1 : -1) : 0;
}

struct Projection {
    const Face &f;
    int k, x, y, sign;
    double ax, ay, low, high;
    explicit Projection(const Face &face) : f(face) {
        V n = cross(sub(f[1], f[0]), sub(f[2], f[0]));
        k = 0;
        for (int i = 1; i < 3; ++i)
            if (std::abs(n[i]) > std::abs(n[k]))
                k = i;
        x = (k + 1) % 3;
        y = (k + 2) % 3;
        sign = turn(f[0], f[1], f[2], x, y);
        ax = n[k] ? std::clamp(-n[x] / n[k], -2., 2.) : 0.;
        ay = n[k] ? std::clamp(-n[y] / n[k], -2., 2.) : 0.;
        const double r = residual(f[1]), s = residual(f[2]);
        low = std::min({0., r, s});
        high = std::max({0., r, s});
    }
    double residual(const V &p) const {
        return (p[k] - f[0][k]) - ax * (p[x] - f[0][x]) - ay * (p[y] - f[0][y]);
    }
    double price(const Face &t) const {
        double a = residual(t[0]), b = residual(t[1]), c = residual(t[2]);
        return std::max(high - std::min({a, b, c}), std::max({a, b, c}) - low);
    }
    double intersection_price(const Plane &p, double limit) const {
        const Face &t = p.f;
        double fast = price(t);
        if (fast <= limit || !p.n[k])
            return fast;
        const double bx = std::clamp(-p.n[x] / p.n[k], -4., 4.);
        const double by = std::clamp(-p.n[y] / p.n[k], -4., 4.);
        double gl = std::numeric_limits<double>::infinity(), gh = -gl;
        for (auto &v : t) {
            double r = (v[k] - f[0][k]) - bx * (v[x] - f[0][x]) - by * (v[y] - f[0][y]);
            gl = std::min(gl, r);
            gh = std::max(gh, r);
        }
        double l = low - gh, h = high - gl;
        for (int dim : {x, y}) {
            double lo = std::max(std::min({f[0][dim], f[1][dim], f[2][dim]}),
                                 std::min({t[0][dim], t[1][dim], t[2][dim]})) -
                        f[0][dim];
            double hi = std::min(std::max({f[0][dim], f[1][dim], f[2][dim]}),
                                 std::max({t[0][dim], t[1][dim], t[2][dim]})) -
                        f[0][dim];
            if (lo > hi)
                return fast;
            double d = dim == x ? ax - bx : ay - by;
            l += std::min(d * lo, d * hi);
            h += std::max(d * lo, d * hi);
        }
        double up = std::min(fast, h), down = std::min(fast, -l);
        for (int e = 0; e < 3 && std::max(up, down) > limit; ++e) {
            V d = sub(f[(e + 1) % 3], f[e]);
            for (int j = 0; j < 3 && std::max(up, down) > limit; ++j) {
                V normal = cross(d, sub(t[(j + 1) % 3], t[j]));
                if (std::abs(normal[k]) <
                        0.125 * std::max(std::abs(normal[x]), std::abs(normal[y])) ||
                    !normal[k])
                    continue;
                const double u = -normal[x] / normal[k], v = -normal[y] / normal[k];
                auto residual = [&](const V &z) {
                    return (z[k] - f[0][k]) - u * (z[x] - f[0][x]) - v * (z[y] - f[0][y]);
                };
                const double a = residual(f[1]), b = residual(f[2]), m = residual(t[0]),
                             n = residual(t[1]), o = residual(t[2]);
                up = std::min(up, std::max({0., a, b}) - std::min({m, n, o}));
                down = std::min(down, std::max({m, n, o}) - std::min({0., a, b}));
            }
        }
        return std::max({0., up, down});
    }
    int side(const V &a, const V &b, const V &c) const { return turn(a, b, c, x, y); }
    bool overlaps(const Face &t, int ts) const {
        for (int i = 0; i < 3; ++i) {
            const int j = (i + 1) % 3;
            if (sign * side(f[i], f[j], t[0]) <= 0 && sign * side(f[i], f[j], t[1]) <= 0 &&
                sign * side(f[i], f[j], t[2]) <= 0)
                return false;
            if (ts * side(t[i], t[j], f[0]) <= 0 && ts * side(t[i], t[j], f[1]) <= 0 &&
                ts * side(t[i], t[j], f[2]) <= 0)
                return false;
        }
        return true;
    }
    bool crosses(const V &a, const V &b) const {
        for (int i = 0; i < 3; ++i) {
            const int j = (i + 1) % 3;
            if (sign * side(f[i], f[j], a) <= 0 && sign * side(f[i], f[j], b) <= 0)
                return false;
        }
        int u = side(a, b, f[0]), v = side(a, b, f[1]), w = side(a, b, f[2]);
        return std::min({u, v, w}) < 0 && std::max({u, v, w}) > 0;
    }
};

struct Neighbors {
    static constexpr U none = UINT32_MAX;
    std::vector<U> across;
    static uint64_t hash(const V &v) {
        uint64_t h = 0;
        for (double value : v) {
            float f = value == 0. ? 0.f : float(value);
            U bits;
            std::memcpy(&bits, &f, sizeof(bits));
            h = (h ^ bits) * 0x9e3779b185ebca87ull;
        }
        return h ^ (h >> 31);
    }
    void build(const std::vector<Plane> &planes, Progress &progress) {
        U count = U(planes.size());
        across.assign(3 * count, none);
        size_t cap = 1;
        while (cap < 5ull * count)
            cap *= 2;
        std::vector<U> table(cap, none);
        for (U i = 0; i < count; ++i) {
            progress.tick();
            const auto &f = planes[i].f;
            const uint64_t hashes[3] = {hash(f[0]), hash(f[1]), hash(f[2])};
            for (U j = 0; j < 3; ++j) {
                U k = (j + 1) % 3;
                uint64_t h = (hashes[j] + hashes[k]) * 0x9ddfea08eb382d69ull;
                size_t slot = (h ^ (h >> 32)) & (cap - 1);
                for (U probes = 0;; slot = (slot + 1) & (cap - 1)) {
                    if (++probes % 128 == 0)
                        progress.tick();
                    U e = table[slot];
                    if (e == none) {
                        table[slot] = 3 * i + j;
                        break;
                    }
                    const auto &g = planes[e / 3].f;
                    U a = e % 3, b = (a + 1) % 3;
                    if ((f[j] == g[a] && f[k] == g[b]) || (f[j] == g[b] && f[k] == g[a])) {
                        if (e / 3 != i && across[e] == none) {
                            across[e] = 3 * i + j;
                            across[3 * i + j] = e;
                        }
                        break;
                    }
                }
            }
        }
    }
};

struct Index {
    std::vector<Plane> planes;
    std::vector<V> centers;
    std::vector<U> order;
    std::vector<Node> nodes;
    Neighbors neighbors;
    double magnitude = 0.;
    V origin{};
    double factor = 0.;
    std::vector<uint64_t> keys;
    static U spread(U n) {
        n = (n | (n << 16)) & 0x030000ffu;
        n = (n | (n << 8)) & 0x0300f00fu;
        n = (n | (n << 4)) & 0x030c30c3u;
        return (n | (n << 2)) & 0x09249249u;
    }
    U code(const V &v) const {
        U k[3];
        for (int j = 0; j < 3; ++j)
            k[j] = spread(U(std::clamp((v[j] - origin[j]) * factor, 0., 1023.)));
        return k[0] | (k[1] << 1) | (k[2] << 2);
    }
    U spot(const V &p) const {
        auto it = std::lower_bound(keys.begin(), keys.end(), uint64_t(code(p)) << 32);
        return U(it == keys.end() ? keys.back() : *it);
    }
    void sort_centers(Progress &progress) {
        origin = centers[0];
        V high = origin;
        for (auto &v : centers)
            for (int k = 0; k < 3; ++k) {
                origin[k] = std::min(origin[k], v[k]);
                high[k] = std::max(high[k], v[k]);
            }
        V ext = sub(high, origin);
        double maximum = *std::max_element(ext.begin(), ext.end());
        factor = maximum > 0 ? 1023. / maximum : 0.;
        keys.reserve(centers.size());
        for (U i = 0; i < centers.size(); ++i)
            keys.push_back((uint64_t(code(centers[i])) << 32) | i);
        // Three stable radix passes produce the same key and face-ID ordering.
        std::vector<uint64_t> scratch(keys.size());
        for (unsigned shift = 32; shift < 62; shift += 10) {
            std::array<U, 1024> counts{};
            for (uint64_t v : keys) {
                ++counts[(v >> shift) & 1023u];
                progress.tick();
            }
            U offset = 0;
            for (U &n : counts) {
                U size = n;
                n = offset;
                offset += size;
            }
            for (uint64_t v : keys) {
                scratch[counts[(v >> shift) & 1023u]++] = v;
                progress.tick();
            }
            keys.swap(scratch);
        }
        for (U i = 0; i < keys.size(); ++i)
            order[i] = U(keys[i]);
    }
    U build(U first, U end, Progress &progress) {
        progress.tick();
        U me = U(nodes.size());
        nodes.emplace_back();
        Node n{};
        n.first = first;
        n.end = end;
        if (end - first > 6) {
            U m = first + (end - first) / 2;
            n.left = build(first, m, progress);
            n.right = build(m, end, progress);
            for (int k = 0; k < 3; ++k) {
                n.lo[k] = std::min(nodes[n.left].lo[k], nodes[n.right].lo[k]);
                n.hi[k] = std::max(nodes[n.left].hi[k], nodes[n.right].hi[k]);
            }
        } else {
            n.lo = n.hi = planes[order[first]].f[0];
            for (U i = first; i < end; ++i)
                for (const V &v : planes[order[i]].f)
                    for (int k = 0; k < 3; ++k) {
                        n.lo[k] = std::min(n.lo[k], v[k]);
                        n.hi[k] = std::max(n.hi[k], v[k]);
                    }
        }
        nodes[me] = n;
        return me;
    }
    double box(V p, U i) const {
        const auto &n = nodes[i];
        double dsq = 0;
        for (int k = 0; k < 3; ++k) {
            double t = std::max({0., p[k] - n.hi[k], n.lo[k] - p[k]});
            dsq += t * t;
        }
        return dsq;
    }
    Index(const Raw *target, U n, Progress &progress) : order(n) {
        planes.reserve(n);
        centers.reserve(n);
        nodes.reserve(n);
        std::iota(order.begin(), order.end(), 0u);
        for (U i = 0; i < n; ++i) {
            progress.tick();
            auto f = lift(target[i]);
            for (const V &v : f)
                for (double value : v) {
                    if (!std::isfinite(value))
                        throw InvalidArithmetic{};
                    magnitude = std::max(magnitude, std::abs(value));
                }
            planes.emplace_back(f);
            centers.push_back(center(f));
        }
        sort_centers(progress);
        build(0, n, progress);
        neighbors.build(planes, progress);
    }
};
class Cover {
    const Index &index;
    Progress &progress;
    std::vector<U> marks;
    U stamp = 0, last = 0;
    uint64_t splits = 0;
    double guide, upper2 = 0., magnitude = 0.;
    bool projected(const Face &f, U seed, double limit, double &height) {
        Projection p(f);
        if (!p.sign)
            return false;
        const auto &s = index.planes[seed].f;
        int sign = p.side(s[0], s[1], s[2]);
        if (!sign || !p.overlaps(s, sign))
            return false;
        constexpr U cap = 1024;
        U todo[cap], read = 0, tail = 1;
        todo[0] = seed;
        if (++stamp == 0) {
            std::fill(marks.begin(), marks.end(), 0);
            ++stamp;
        }
        marks[seed] = stamp;
        height = 0;
        while (read < tail) {
            U i = todo[read++];
            const auto &t = index.planes[i].f;
            height = std::max(height, p.intersection_price(index.planes[i], limit));
            if (height > limit)
                return false;
            progress.tick();
            for (U a = 0; a < 3; ++a) {
                U b = (a + 1) % 3, e = index.neighbors.across[3 * i + a];
                if (e != Neighbors::none && marks[e / 3] == stamp) {
                    // A visited neighbor still needs to cover the opposite side.
                    const auto &q = index.planes[e / 3].f[(e % 3 + 2) % 3];
                    if (p.side(t[a], t[b], q) * p.side(t[a], t[b], t[(a + 2) % 3]) < 0)
                        continue;
                }
                if (!p.crosses(t[a], t[b]))
                    continue;
                if (e == Neighbors::none)
                    return false;
                const auto &q = index.planes[e / 3].f[(e % 3 + 2) % 3];
                if (p.side(t[a], t[b], q) * p.side(t[a], t[b], t[(a + 2) % 3]) >= 0)
                    return false;
                if (marks[e / 3] != stamp) {
                    if (tail == cap)
                        return false;
                    marks[e / 3] = stamp;
                    todo[tail++] = e / 3;
                }
            }
        }
        return true;
    }

    U walk(const Projection &p, const V &v, U seed) const {
        U previous = Neighbors::none;
        for (int step = 0; step < 12; ++step) {
            const auto &t = index.planes[seed].f;
            double n = index.planes[seed].n[p.k];
            if (n == 0.)
                break;
            double worst = 0.;
            U edge = 3;
            for (U j = 0; j < 3; ++j) {
                const V &a = t[j], &b = t[(j + 1) % 3];
                double w =
                    (b[p.x] - a[p.x]) * (v[p.y] - a[p.y]) - (b[p.y] - a[p.y]) * (v[p.x] - a[p.x]);
                if (n < 0.)
                    w = -w;
                if (w < worst) {
                    worst = w;
                    edge = j;
                }
            }
            if (edge == 3)
                break;
            U e = index.neighbors.across[3 * seed + edge];
            if (e == Neighbors::none || e / 3 == previous)
                break;
            previous = seed;
            seed = e / 3;
        }
        return seed;
    }

    void search(V p, U i, Witness &w, int &budget) {
        if (budget == 0 || w.dsq <= guide * 0.0025)
            return;
        --budget;
        const auto &n = index.nodes[i];
        if (!n.left) {
            for (U j = n.first; j < n.end; ++j) {
                U id = index.order[j];
                double d = index.planes[id].choose(p).dsq;
                if (d < w.dsq)
                    w = {id, d};
            }
        } else {
            U a = n.left, b = n.right;
            double da = index.box(p, a), db = index.box(p, b);
            if (da > db) {
                std::swap(a, b);
                std::swap(da, db);
            }
            if (da < w.dsq)
                search(p, a, w, budget);
            if (db < w.dsq)
                search(p, b, w, budget);
        }
    }
    Witness near(V p, U seed) {
        progress.tick();
        Witness w{seed, index.planes[seed].choose(p).dsq};
        int budget = 768;
        search(p, 0, w, budget);
        return w;
    }
    static double plane_lower(const Plane &t, const V &p, double pad) {
        Choice w = t.choose(p);
        V dir = sub(sub(p, t.f[0]), add(scale(t.e, w.u), scale(t.g, w.v)));
        double largest = std::max({std::abs(dir[0]), std::abs(dir[1]), std::abs(dir[2])});
        if (!(largest > 0.) || !std::isfinite(largest))
            return 0.;
        for (double &v : dir)
            v /= largest;
        double length = std::sqrt(dot(dir, dir)) * (1. + 0x1p-47);
        double h = std::min(
            {dot(dir, sub(p, t.f[0])), dot(dir, sub(p, t.f[1])), dot(dir, sub(p, t.f[2]))});
        return (h - pad) / length - pad;
    }
    bool separated(V p, U i, double radius, double pad, int &budget) {
        if (--budget <= 0)
            return false;
        if (std::sqrt(index.box(p, i)) - pad >= radius)
            return true;
        const auto &h = index.nodes[i];
        if (h.left)
            return separated(p, h.left, radius, pad, budget) &&
                   separated(p, h.right, radius, pad, budget);
        for (U j = h.first; j < h.end; ++j)
            if (plane_lower(index.planes[index.order[j]], p, pad) < radius)
                return false;
        return true;
    }
    U lower_attempts = 0;
    void observe(const V &p, double dsq) {
        if (!(dsq * 1.0201 > guide) || lower_attempts >= 192)
            return;
        ++lower_attempts;
        // A small centroid may result from cancellation of large domain corners.
        // The scale includes every corner in this range and in the target.
        double pad = std::ldexp(magnitude, -42);
        // The unused sample slack pays for query rounding at domain centroids.
        double distance = std::sqrt(dsq) * (1. - 0x1p-16) - 4 * pad;
        int budget = 3072;
        if (distance > 0. && separated(p, 0, distance + pad, pad, budget))
            guide = std::max(guide, 1.0201 * distance * distance);
    }
    Patch patch(const Face &f, U id) const {
        Patch p{id, {}, 0};
        for (int i = 0; i < 3; ++i) {
            p.weights[i] = index.planes[id].choose(f[i]);
            p.price = std::max(p.price, p.weights[i].dsq);
        }
        return p;
    }
    void emit(const Face &f, const Patch &p) {
        const Face &t = index.planes[p.id].f;
        for (int i = 0; i < 3; ++i) {
            const auto &w = p.weights[i];
            auto snap = [](double x) -> U {
                return std::isfinite(x) ? U(std::clamp(x, 0., 1.) * Denominator + 0.5) : 0;
            };
            U u = snap(w.u), v = std::min(Denominator - u, snap(w.v));
            double a = double(Denominator - u - v) / Denominator, b = double(u) / Denominator,
                   c = double(v) / Denominator;
            V q = add(add(scale(t[0], a), scale(t[1], b)), scale(t[2], c));
            V delta = sub(f[i], q);
            upper2 = std::max(upper2, dot(delta, delta));
            guide = std::max(guide, upper2);
        }
        progress.tick();
    }
    void cover(const Face &f, U hint, int depth, int budget, bool seeded = false) {
        Patch p = patch(f, hint);
        if (seeded && p.price > guide) {
            double price;
            if (projected(f, hint, std::sqrt(guide), price)) {
                last = hint;
                upper2 = std::max(upper2, price * price);
                return;
            }
        }
        if (depth == 0 && p.price > guide) {
            V c = center(f);
            U seed = index.spot(c);
            const V a = sub(c, index.centers[hint]), b = sub(c, index.centers[seed]);
            if (dot(a, a) < dot(b, b))
                seed = hint;
            const Projection local(f);
            seed = walk(local, c, seed);
            if (seed != hint) {
                Patch q = patch(f, seed);
                if (q.price < p.price) {
                    p = q;
                    hint = seed;
                }
            }
            if (p.price > guide) {
                double value;
                if (projected(f, seed, std::sqrt(guide), value)) {
                    last = seed;
                    upper2 = std::max(upper2, value * value);
                    return;
                }
            }
        }
        if (p.price > guide) {
            Witness h = near(center(f), hint);
            observe(center(f), h.dsq);
            if (h.id != hint) {
                Patch q = patch(f, h.id);
                if (q.price < p.price)
                    p = q;
            }
            if (p.price > guide && (depth == MaxDepth || budget == 0)) {
                for (auto vertex : f) {
                    Witness w = near(vertex, p.id);
                    observe(vertex, w.dsq);
                    Patch q = patch(f, w.id);
                    if (q.price < p.price)
                        p = q;
                }
            }
        }
        if (depth == 0)
            last = p.id;
        if (depth < 6 && p.price > guide) {
            Face raw = f;
            double displacement = 0.;
            if (depth) {
                for (int i = 0; i < 3; ++i) {
                    for (int k = 0; k < 3; ++k)
                        raw[i][k] = float(f[i][k]);
                    V d = sub(f[i], raw[i]);
                    displacement = std::max(displacement, dot(d, d));
                }
                displacement = std::sqrt(displacement);
            }
            double price, goal = std::sqrt(guide) - displacement;
            if (goal >= 0. && projected(raw, p.id, goal, price)) {
                price += displacement;
                upper2 = std::max(upper2, price * price);
                return;
            }
        }
        if (p.price <= guide || depth == MaxDepth || budget == 0) {
            emit(f, p);
            return;
        }
        ++splits;
        V a = mid(f[0], f[1]), b = mid(f[1], f[2]), c = mid(f[2], f[0]);
        // Split credits are assigned across all four panels for balanced effort.
        int share = (budget - 1) / 4, extra = (budget - 1) % 4;
        cover({f[0], a, c}, p.id, depth + 1, share + (extra > 0));
        cover({a, f[1], b}, p.id, depth + 1, share + (extra > 1));
        cover({c, b, f[2]}, p.id, depth + 1, share + (extra > 2));
        cover({a, b, c}, p.id, depth + 1, share);
    }

  public:
    Cover(const Index &target, Progress &work, double goal)
        : index(target), progress(work), marks(target.planes.size()), guide(goal * goal) {}
    double run(const Raw *domain, const U *hints, U n, uint64_t credits) {
        magnitude = index.magnitude;
        for (U i = 0; i < n; ++i) {
            progress.tick();
            for (const auto &v : domain[i])
                for (float value : v) {
                    if (!std::isfinite(value))
                        throw InvalidArithmetic{};
                    magnitude = std::max(magnitude, std::abs(double(value)));
                }
        }
        // A complete depth-seven split fits 5,461 splits per face. Usually the
        // parent-face test stops most branches much earlier. Bound exceptional
        // directions globally; when credits run short, retain a valid coarse cover.

        for (U i = 0; i < n; ++i) {
            uint64_t before = splits;
            int budget = static_cast<int>(std::min<uint64_t>(5461, credits));
            const bool seeded = hints && hints[i] < index.planes.size();
            cover(lift(domain[i]), seeded ? hints[i] : last, 0, budget, seeded);
            credits -= splits - before;
            ++progress.pending;
        }
        // The largest height slope used by an accepted support plane is eight.
        // The documented outward allowance covers both witnesses and the panel.
        return std::nextafter(std::sqrt(upper2) + std::ldexp(magnitude, -42),
                              std::numeric_limits<double>::infinity());
    }
};

template <typename Work> int guarded(Work work) noexcept {
    try {
        check_arithmetic();
        work();
        return 0;
    } catch (const Interrupted &) {
        return 3;
    } catch (const std::bad_alloc &) {
        return 2;
    } catch (...) {
        return 1;
    }
}
} // namespace
extern "C" int rt_cert_index(const Raw *faces, U n, void **out, Callback cb, void *ctx) noexcept {
    if (!faces || !n || n > UINT32_MAX / 3 || !out || !cb)
        return 1;
    return guarded([&] {
        Progress progress{cb, ctx};
        std::unique_ptr<Index> index(new Index(faces, n, progress));
        progress.flush();
        *out = index.release();
    });
}
extern "C" void rt_cert_free(void *ptr) noexcept { delete static_cast<Index *>(ptr); }
extern "C" int rt_cert_range(const void *ptr, const Raw *domain, const U *hints, U n,
                             uint64_t credits, double goal, double *upper, Callback cb,
                             void *ctx) noexcept {
    if (!ptr || !domain || !n || !cb || !upper || !std::isfinite(goal) || goal < 0.)
        return 1;
    return guarded([&] {
        Progress progress{cb, ctx};
        const auto &index = *static_cast<const Index *>(ptr);
        *upper = Cover(index, progress, goal).run(domain, hints, n, credits);
        progress.flush();
        if (!std::isfinite(*upper))
            throw InvalidArithmetic{};
    });
}
// The residual allowance and the Rust ABI require these arithmetic formats.
static_assert(std::numeric_limits<float>::is_iec559 && std::numeric_limits<float>::digits == 24);
static_assert(std::numeric_limits<double>::is_iec559 && std::numeric_limits<double>::digits == 53);
static_assert(sizeof(Raw) == 9 * sizeof(float));
#if defined(__FAST_MATH__) || (defined(__FINITE_MATH_ONLY__) && __FINITE_MATH_ONLY__ > 0)
#error "The surface certificate requires IEEE floating-point semantics"
#endif
