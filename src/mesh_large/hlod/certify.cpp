// Interior-complete two-sided distance certificate. Approximate geometry chooses a
// witness only. Its three snapped simplex coordinates and residuals bound the
// entire closed panel, including degenerate triangles, by convex interpolation.
#include <algorithm>
#include <array>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <limits>
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
    double nn, ie, ig, ib;
    explicit Plane(Face t)
        : f(t), e(sub(f[1], f[0])), g(sub(f[2], f[0])), n(cross(e, g)), bc(sub(f[2], f[1])),
          nn(dot(n, n)) {
        auto inv = [](double d) { return d > 0 ? 1. / d : 0.; };
        du = scale(cross(g, n), inv(nn));
        dv = scale(cross(n, e), inv(nn));
        ie = inv(dot(e, e));
        ig = inv(dot(g, g));
        ib = inv(dot(bc, bc));
    }
    Choice choose(V p) const {
        V t = sub(p, f[0]);
        double u = dot(t, du), v = dot(t, dv);
        if (nn > 0 && u >= 0 && v >= 0 && u + v <= 1) {
            double h = dot(t, n);
            return {u, v, h * h / nn};
        }
        Choice best{0, 0, dot(t, t)};
        auto edge = [&](V a, V dir, double inv, int k) {
            double l = std::clamp(dot(a, dir) * inv, 0., 1.);
            V d = sub(a, scale(dir, l));
            double dsq = dot(d, d);
            if (dsq < best.dsq)
                best = {k == 0 ? l : k == 1 ? 1 - l : 0., k == 1 ? l : k == 2 ? l : 0., dsq};
        };
        edge(t, e, ie, 0);
        edge(sub(p, f[1]), bc, ib, 1);
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
static_assert(sizeof(Plane) + sizeof(V) + sizeof(U) + sizeof(Node) + sizeof(Raw) <= 512);
struct Witness {
    U id;
    double dsq;
};
struct Patch {
    U id;
    std::array<Choice, 3> weights;
    double price;
};
class Direction {
    std::vector<Plane> planes;
    std::vector<V> centers;
    std::vector<U> order;
    std::vector<Node> nodes;
    Progress &progress;
    uint64_t splits = 0;
    double guide, upper2 = 0.;
    U build(U first, U end) {
        const V point = planes[order[first]].f[0];
        Node n{point, point, first, end};
        for (U i = first; i < end; ++i) {
            progress.tick();
            for (V p : planes[order[i]].f)
                for (int k = 0; k < 3; ++k) {
                    n.lo[k] = std::min(n.lo[k], p[k]);
                    n.hi[k] = std::max(n.hi[k], p[k]);
                }
        }
        U me = static_cast<U>(nodes.size());
        nodes.push_back(n);
        if (end - first > 6) {
            V d = sub(n.hi, n.lo);
            int k = std::max_element(d.begin(), d.end()) - d.begin();
            U m = first + (end - first) / 2;
            std::nth_element(order.begin() + first, order.begin() + m, order.begin() + end,
                             [&](U a, U b) {
                                 progress.tick();
                                 return centers[a][k] < centers[b][k];
                             });
            U l = build(first, m), r = build(m, end);
            nodes[me].left = l;
            nodes[me].right = r;
        }
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
    void search(V p, U i, Witness &w, int &budget) {
        if (budget == 0)
            return;
        --budget;
        const auto &n = nodes[i];
        if (!n.left) {
            for (U j = n.first; j < n.end; ++j) {
                U id = order[j];
                double d = planes[id].choose(p).dsq;
                if (d < w.dsq)
                    w = {id, d};
            }
        } else {
            U a = n.left, b = n.right;
            double da = box(p, a), db = box(p, b);
            if (da > db) {
                std::swap(a, b);
                std::swap(da, db);
            }
            if (da <= w.dsq)
                search(p, a, w, budget);
            if (db <= w.dsq)
                search(p, b, w, budget);
        }
    }
    Witness near(V p, U seed) {
        progress.tick();
        Witness w{seed, planes[seed].choose(p).dsq};
        int budget = 768;
        search(p, 0, w, budget);
        return w;
    }
    Patch patch(const Face &f, U id) const {
        Patch p{id, {}, 0};
        for (int i = 0; i < 3; ++i) {
            p.weights[i] = planes[id].choose(f[i]);
            p.price = std::max(p.price, p.weights[i].dsq);
        }
        return p;
    }
    void emit(const Face &f, const Patch &p) {
        const Face &t = planes[p.id].f;
        for (int i = 0; i < 3; ++i) {
            const auto &w = p.weights[i];
            auto snap = [](double x) -> U {
                return std::isfinite(x) ? U(std::round(std::clamp(x, 0., 1.) * Denominator)) : 0;
            };
            U u = snap(w.u), v = std::min(Denominator - u, snap(w.v));
            double a = double(Denominator - u - v) / Denominator, b = double(u) / Denominator,
                   c = double(v) / Denominator;
            V q = add(add(scale(t[0], a), scale(t[1], b)), scale(t[2], c));
            V delta = sub(f[i], q);
            upper2 = std::max(upper2, dot(delta, delta));
        }
        progress.tick();
    }
    void cover(const Face &f, U hint, int depth, int budget) {
        Patch p = patch(f, hint);
        if (p.price > guide) {
            Witness h = near(center(f), hint);
            guide = std::max(guide, h.dsq * 1.030225);
            if (h.id != hint) {
                Patch q = patch(f, h.id);
                if (q.price < p.price)
                    p = q;
            }
            if (p.price > guide && (depth == MaxDepth || budget == 0)) {
                for (auto vertex : f) {
                    Witness w = near(vertex, p.id);
                    guide = std::max(guide, w.dsq * 1.030225);
                    Patch q = patch(f, w.id);
                    if (q.price < p.price)
                        p = q;
                }
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
    Direction(const Raw *target, U n, double goal, Progress &work)
        : order(n), progress(work), guide(goal * goal) {
        planes.reserve(n);
        centers.reserve(n);
        nodes.reserve(n);
        std::iota(order.begin(), order.end(), 0u);
        for (U i = 0; i < n; ++i) {
            progress.tick();
            auto f = lift(target[i]);
            planes.emplace_back(f);
            centers.push_back(center(f));
        }
        build(0, n);
    }
    double run(const Raw *domain, U n, U target_count) {
        U hint = 0;
        // Seed the work threshold before expensive face covers. Vertex samples
        // steer work only; every domain face is still covered in the second pass.
        std::vector<U> seeds(n);
        for (U i = 0; i < n; ++i) {
            auto f = lift(domain[i]);
            auto w = near(center(f), hint);
            seeds[i] = hint = w.id;
            guide = std::max(guide, w.dsq * 1.030225);
            for (V v : f)
                guide = std::max(guide, near(v, hint).dsq * 1.030225);
        }
        // A complete depth-seven split fits 5,461 splits per face. Usually the
        // parent-face test stops most branches much earlier. Bound exceptional
        // directions globally; when credits run short, retain a valid coarse cover.
        uint64_t credits = std::max<uint64_t>(65536, 256ull * std::max(n, target_count));
        for (U i = 0; i < n; ++i) {
            uint64_t before = splits;
            int budget = static_cast<int>(std::min<uint64_t>(5461, credits));
            cover(lift(domain[i]), seeds[i], 0, budget);
            credits -= splits - before;
            ++progress.pending;
        }
        return std::sqrt(upper2);
    }
};
} // namespace

extern "C" int rt_surface_cert(const Raw *a, uint32_t na, const Raw *b, uint32_t nb, double goal,
                               double *forward, double *backward, Callback callback,
                               void *context) noexcept {
    if (!a || !b || !na || !nb || !callback || !std::isfinite(goal) || goal < 0)
        return 1;
    try {
        // Host applications may set rounding or flush binary32 subnormals.
        // Refuse those modes before lifting the encoded coordinates.
        check_arithmetic();
        Progress progress{callback, context};
        double magnitude = 0.;
        auto check = [&](const Raw *faces, U n) {
            for (U i = 0; i < n; ++i) {
                progress.tick();
                for (auto &v : faces[i])
                    for (float x : v) {
                        if (!std::isfinite(x))
                            return false;
                        magnitude = std::max(magnitude, std::abs(double(x)));
                    }
            }
            return true;
        };
        if (!check(a, na) || !check(b, nb))
            return 1;
        // With <=7 midpoint levels, simplex weights on a 2^-22 grid, binary32
        // inputs, and IEEE binary64, 512 unit roundoffs times max |coordinate|
        // covers panel, witness and L2 arithmetic (see docs/architecture/hlod-s2-error.md).
        double pad = std::ldexp(magnitude, -44);
        auto outward = [&](double x) {
            return std::nextafter(x + pad, std::numeric_limits<double>::infinity());
        };
        *forward = outward(Direction(b, nb, goal, progress).run(a, na, nb));
        *backward = outward(Direction(a, na, goal, progress).run(b, nb, na));
        progress.flush();
        return std::isfinite(*forward) && std::isfinite(*backward) ? 0 : 1;
    } catch (const Interrupted &) {
        return 3;
    } catch (const std::bad_alloc &) {
        return 2;
    } catch (...) {
        return 1;
    }
}

// The residual allowance and the Rust ABI require these arithmetic formats.
static_assert(std::numeric_limits<float>::is_iec559 && std::numeric_limits<float>::digits == 24);
static_assert(std::numeric_limits<double>::is_iec559 && std::numeric_limits<double>::digits == 53);
static_assert(sizeof(Raw) == 9 * sizeof(float));
#if defined(__FAST_MATH__) || (defined(__FINITE_MATH_ONLY__) && __FINITE_MATH_ONLY__ > 0)
#error "The surface certificate requires IEEE floating-point semantics"
#endif
