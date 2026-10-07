//! Shared 3D Tiles node pieces for hierarchies whose children sit at a pure
//! translation from their parent's centre, with origin-centred box bounds.
use crate::{vec3::Vec3, Error};
use serde_json::Value;

/// A translation-only column-major `transform`.
pub(crate) fn translation(delta: Vec3) -> Value {
    serde_json::json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, delta[0], delta[1], delta[2], 1])
}

/// The offset of a node written with [`translation`].
pub(crate) fn translation_offset(node: &Value) -> Result<Vec3, Error> {
    let mut offset = [0.; 3];
    for (i, value) in offset.iter_mut().enumerate() {
        *value = node["transform"][12 + i]
            .as_f64()
            .ok_or_else(|| Error::Data("invalid child transform".into()))?;
    }
    Ok(offset)
}

/// An axis-aligned, origin-centred `box`. `zero` is written verbatim so each
/// converter keeps its published JSON form (`0` for vector, `0.0` for points).
pub(crate) fn box_json(zero: impl Into<Value>, half: Vec3) -> Value {
    let z: Value = zero.into();
    Value::Array(vec![
        z.clone(),
        z.clone(),
        z.clone(),
        half[0].into(),
        z.clone(),
        z.clone(),
        z.clone(),
        half[1].into(),
        z.clone(),
        z.clone(),
        z,
        half[2].into(),
    ])
}

/// Half extents of an axis-aligned `box` value.
pub(crate) fn box_half(value: &Value) -> Result<Vec3, Error> {
    let mut half = [0.; 3];
    for (i, h) in half.iter_mut().enumerate() {
        *h = value[3 + i * 4]
            .as_f64()
            .ok_or_else(|| Error::Data("invalid child bounds".into()))?;
    }
    Ok(half)
}

/// Grow `half` (the parent's own, already padded content box) to enclose
/// every child placed at `delta`, and return the largest of `error` and the
/// children's geometric errors. Children carry their own padding, so the
/// enclosure stays conservative. Malformed child bounds are data errors.
pub(crate) fn enclose_children<'a>(
    half: &mut Vec3,
    mut error: f64,
    children: impl IntoIterator<Item = (Vec3, &'a Value)>,
) -> Result<f64, Error> {
    for (delta, child) in children {
        let child_half = box_half(&child["boundingVolume"]["box"])?;
        for i in 0..3 {
            half[i] = half[i].max(delta[i].abs() + child_half[i]);
        }
        error = error.max(child["geometricError"].as_f64().unwrap_or(0.));
    }
    Ok(error)
}

/// Tileset-level geometric error: at least 1 m, the root box `diagonal`, and
/// `root_margin` times the root's own error. A margin above 1 is a converter
/// policy that leaves an SSE interval in which the root itself renders.
pub(crate) fn top_level_error(diagonal: f64, root_error: f64, root_margin: f64) -> f64 {
    1f64.max(root_error * root_margin).max(diagonal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_forms_match_published_tilesets() {
        let half = [1.5, 2., 0.25];
        assert_eq!(
            box_json(0, half).to_string(),
            json!([0, 0, 0, 1.5, 0, 0, 0, 2., 0, 0, 0, 0.25]).to_string()
        );
        assert_eq!(
            box_json(0., half).to_string(),
            json!([0., 0., 0., 1.5, 0., 0., 0., 2., 0., 0., 0., 0.25]).to_string()
        );
        let node = json!({"transform": translation([1., -2., 3.5])});
        assert_eq!(
            node["transform"].to_string(),
            "[1,0,0,0,0,1,0,0,0,0,1,0,1.0,-2.0,3.5,1]"
        );
        assert_eq!(translation_offset(&node).unwrap(), [1., -2., 3.5]);
    }

    #[test]
    fn children_grow_parent_bounds_and_errors_or_are_rejected() {
        let child = json!({"boundingVolume":{"box":box_json(0, [1., 1., 1.])},"geometricError":4.});
        let mut half = [0.5; 3];
        let error = enclose_children(&mut half, 2., [([-3., 0., 0.25], &child)]).unwrap();
        assert_eq!((half, error), ([4., 1., 1.25], 4.));
        let broken = json!({"boundingVolume":{"box":[0, 0, 0]}});
        assert!(matches!(
            enclose_children(&mut half, 0., [([0.; 3], &broken)]),
            Err(Error::Data(_))
        ));
        assert_eq!(top_level_error(0.5, 0.25, 2.), 1.);
        assert_eq!(top_level_error(3., 2., 2.), 4.);
    }
}
