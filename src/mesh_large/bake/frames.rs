//! The pinned authored Cesium TBN path preserves source tangent/normal skew.
use super::{
    math::{self, V3},
    unsupported,
};
use crate::JobError;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub tangent: V3,
    pub bitangent: V3,
    pub normal: V3,
}
impl Frame {
    pub(crate) fn from_triangle(
        p: [V3; 3],
        uv: [[f64; 2]; 3],
        normal: V3,
    ) -> Result<Self, JobError> {
        let n = math::unit(normal).ok_or_else(|| unsupported("proxy normal has no direction"))?;
        let e1 = math::sub(p[1], p[0]);
        let e2 = math::sub(p[2], p[0]);
        let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
        let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
        let det = a[0] * b[1] - a[1] * b[0];
        if det == 0.0 || !det.is_finite() {
            return Err(unsupported(
                "nondegenerate proxy has singular decoded UV frame",
            ));
        }
        // Common division magnitude is irrelevant after direction normalization.
        let t = math::mul(
            math::sub(math::mul(e1, b[1]), math::mul(e2, a[1])),
            det.signum(),
        );
        let raw_b = math::mul(
            math::sub(math::mul(e2, a[0]), math::mul(e1, b[0])),
            det.signum(),
        );
        let t = math::unit(math::sub(t, math::mul(n, math::dot(n, t))))
            .ok_or_else(|| unsupported("proxy tangent projection has no direction"))?;
        let cross = math::cross(n, t);
        let w = if math::dot(cross, raw_b) < 0.0 {
            -1.0
        } else {
            1.0
        };
        Ok(Self {
            tangent: t,
            bitangent: math::mul(cross, w),
            normal: n,
        })
    }
    /// GeometryStageVS and GeometryStageFS normalize each axis separately.
    /// Neither orthogonalizes authored T/N; MaterialStageFS then multiplies TBN.
    pub(crate) fn donor(
        normals: [V3; 3],
        tangents: [[f64; 4]; 3],
        w: V3,
    ) -> Result<Self, JobError> {
        let mut ns = [[0.0; 3]; 3];
        let mut ts = ns;
        let mut bs = ns;
        for i in 0..3 {
            ns[i] = math::unit(normals[i])
                .ok_or_else(|| unsupported("source normal has no direction"))?;
            ts[i] = math::unit([tangents[i][0], tangents[i][1], tangents[i][2]])
                .ok_or_else(|| unsupported("source tangent has no direction"))?;
            bs[i] = math::unit(math::mul(math::cross(ns[i], ts[i]), tangents[i][3]))
                .ok_or_else(|| unsupported("source authored TBN is singular"))?;
        }
        Self::interpolate(ns, ts, bs, w)
    }
    pub(crate) fn interpolate(
        ns: [V3; 3],
        ts: [V3; 3],
        bs: [V3; 3],
        w: V3,
    ) -> Result<Self, JobError> {
        Ok(Self {
            normal: math::unit(math::bary(ns, w))
                .ok_or_else(|| unsupported("interpolated normal cancelled"))?,
            tangent: math::unit(math::bary(ts, w))
                .ok_or_else(|| unsupported("interpolated tangent cancelled"))?,
            bitangent: math::unit(math::bary(bs, w))
                .ok_or_else(|| unsupported("interpolated bitangent cancelled"))?,
        })
    }
    pub(crate) fn transform(self, v: V3) -> Result<V3, JobError> {
        math::unit(math::add(
            math::add(
                math::mul(self.tangent, v[0]),
                math::mul(self.bitangent, v[1]),
            ),
            math::mul(self.normal, v[2]),
        ))
        .ok_or_else(|| unsupported("effective source shading normal cancelled"))
    }
    /// Full cofactor inverse, preserving skew and determinant sign.
    /// Normalize before division to avoid overflow for nearly singular frames.
    pub(crate) fn inverse_direction(self, v: V3) -> Result<V3, JobError> {
        let bc = math::cross(self.bitangent, self.normal);
        let det = math::dot(self.tangent, bc);
        if det == 0.0 || !det.is_finite() {
            return Err(unsupported("decoded proxy TBN is singular"));
        }
        let q = [
            math::dot(bc, v),
            math::dot(math::cross(self.normal, self.tangent), v),
            math::dot(math::cross(self.tangent, self.bitangent), v),
        ];
        math::unit(math::mul(q, det.signum()))
            .ok_or_else(|| unsupported("normal frame inversion has no direction"))
    }
}
pub(crate) fn reexpress(rgb: V3, scale: f64, donor: Frame, proxy: Frame) -> Result<V3, JobError> {
    if !scale.is_finite()
        || rgb
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(unsupported("normal sample or literal scale is not finite"));
    }
    let v = rgb.map(|v| 2.0 * v - 1.0);
    proxy.inverse_direction(donor.transform(effective_direction(v, scale)?)?)
}
pub(super) fn effective_direction(v: V3, scale: f64) -> Result<V3, JobError> {
    if !scale.is_finite() || v.iter().any(|v| !v.is_finite()) {
        return Err(unsupported("normal signal/literal scale is not finite"));
    }
    let k = scale.abs();
    let effective = if k > 1.0 {
        [v[0] * scale.signum(), v[1] * scale.signum(), v[2] / k]
    } else {
        [v[0] * scale, v[1] * scale, v[2]]
    };
    math::unit(effective).ok_or_else(|| unsupported("source effective normal is zero"))
}
