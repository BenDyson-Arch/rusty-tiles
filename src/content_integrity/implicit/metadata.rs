//! §5 metadata: class classification once per `(schema document, class)`
//! (DC-4) and property-table admission, O(1 + the table's own properties).
use super::subtree::Storage;
use super::{invalid, uint, unsupported, FormatError};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const TILE_BOUNDING_BOX: &str = "TILE_BOUNDING_BOX";
const TILE_GEOMETRIC_ERROR: &str = "TILE_GEOMETRIC_ERROR";

/// Unsupported in any class used by a walked table or a template header (RC-1).
const UNSUPPORTED: [&str; 15] = [
    "TILE_BOUNDING_SPHERE",
    "TILE_BOUNDING_REGION",
    "TILE_BOUNDING_S2_CELL",
    "TILE_MINIMUM_HEIGHT",
    "TILE_MAXIMUM_HEIGHT",
    "TILE_TRANSFORM",
    "TILE_REFINE",
    "CONTENT_URI",
    "CONTENT_GROUP_ID",
    "CONTENT_BOUNDING_BOX",
    "CONTENT_BOUNDING_REGION",
    "CONTENT_BOUNDING_SPHERE",
    "CONTENT_BOUNDING_S2_CELL",
    "CONTENT_MINIMUM_HEIGHT",
    "CONTENT_MAXIMUM_HEIGHT",
];

/// The metadata schema of one schema document (the inline `schema` of a
/// tileset JSON, or one `schemaUri` member). Callers keep one value per schema
/// document for the whole validation, so each class is classified once.
#[derive(Debug)]
pub(crate) struct ImplicitSchema {
    classes: Option<Map<String, Value>>,
    records: RefCell<HashMap<String, Rc<ClassRecord>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Use {
    Tile,
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Float {
    F32,
    F64,
}

impl Float {
    pub(super) fn size(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::F64 => 8,
        }
    }
    pub(super) fn read(self, bytes: &[u8]) -> f64 {
        match self {
            Self::F32 => f64::from(f32::from_le_bytes(bytes[..4].try_into().unwrap())),
            Self::F64 => f64::from_le_bytes(bytes[..8].try_into().unwrap()),
        }
    }
}

/// A deferred verdict, reported when the class is used.
#[derive(Clone, Debug)]
enum Defect {
    Invalid(String),
    Unsupported(String),
}

impl Defect {
    fn error(&self) -> FormatError {
        match self {
            Self::Invalid(message) => invalid(message),
            Self::Unsupported(reason) => unsupported(reason),
        }
    }
}

/// An interpreted tile property (TILE_BOUNDING_BOX or TILE_GEOMETRIC_ERROR).
#[derive(Debug)]
struct Interpreted {
    id: String,
    float: Float,
    required: bool,
}

/// The single classification scan of one class (§5, DC-4).
#[derive(Debug)]
pub(super) struct ClassRecord {
    /// Malformed class structure or a duplicated semantic: InvalidInput.
    malformed: Option<String>,
    /// The first semantic of the Unsupported set.
    unsupported: Option<&'static str>,
    /// TILE_BOUNDING_BOX or TILE_GEOMETRIC_ERROR, Unsupported in a content class.
    tile_semantic: Option<&'static str>,
    bounding_box: Option<Result<Interpreted, Defect>>,
    geometric_error: Option<Result<Interpreted, Defect>>,
}

#[cfg(test)]
thread_local! {
    pub(super) static CLASS_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl ImplicitSchema {
    pub(crate) fn new(schema: Value) -> Self {
        let classes = match schema {
            Value::Object(mut schema) => match schema.remove("classes") {
                Some(Value::Object(classes)) => Some(classes),
                _ => None,
            },
            _ => None,
        };
        Self {
            classes,
            records: RefCell::new(HashMap::new()),
        }
    }

    /// The class's record for `use_`: classified on first use, then reused.
    pub(super) fn admit(&self, class: &str, use_: Use) -> Result<Rc<ClassRecord>, FormatError> {
        let cached = self.records.borrow().get(class).cloned();
        let record = match cached {
            Some(record) => record,
            None => {
                let value = self
                    .classes
                    .as_ref()
                    .and_then(|c| c.get(class))
                    .ok_or_else(|| invalid(format!("unknown metadata class {class:?}")))?;
                let record = Rc::new(classify(value));
                let mut records = self.records.borrow_mut();
                records.try_reserve(1).map_err(|_| super::refused())?;
                records.insert(class.to_owned(), record.clone());
                record
            }
        };
        if let Some(message) = &record.malformed {
            return Err(invalid(format!("metadata class {class:?}: {message}")));
        }
        if let Some(semantic) = record.unsupported {
            return Err(unsupported(format!(
                "semantic {semantic} in class {class:?}"
            )));
        }
        match use_ {
            Use::Content => {
                if let Some(semantic) = record.tile_semantic {
                    return Err(unsupported(format!(
                        "semantic {semantic} in content class {class:?}"
                    )));
                }
            }
            Use::Tile => {
                for property in [&record.bounding_box, &record.geometric_error] {
                    if let Some(Err(defect)) = property {
                        return Err(defect.error());
                    }
                }
            }
        }
        Ok(record)
    }
}

fn classify(class: &Value) -> ClassRecord {
    #[cfg(test)]
    CLASS_SCANS.with(|n| n.set(n.get() + 1));
    let mut record = ClassRecord {
        malformed: None,
        unsupported: None,
        tile_semantic: None,
        bounding_box: None,
        geometric_error: None,
    };
    let properties = match class.as_object().map(|c| c.get("properties")) {
        Some(None) => return record,
        Some(Some(Value::Object(properties))) => properties,
        _ => {
            record.malformed = Some("class and its properties must be objects".into());
            return record;
        }
    };
    let mut seen = HashSet::new();
    for (id, property) in properties {
        let Some(property) = property.as_object() else {
            record.malformed = Some(format!("property {id:?} must be an object"));
            return record;
        };
        let semantic = match property.get("semantic") {
            None => continue,
            Some(Value::String(semantic)) => semantic.as_str(),
            Some(_) => {
                record.malformed = Some(format!("property {id:?} semantic must be a string"));
                return record;
            }
        };
        if seen.try_reserve(1).is_err() {
            record.malformed = Some("classification storage allocation refused".into());
            return record;
        }
        if !seen.insert(semantic) {
            record.malformed = Some(format!("duplicated semantic {semantic}"));
            return record;
        }
        if let Some(name) = UNSUPPORTED.iter().find(|s| **s == semantic) {
            record.unsupported.get_or_insert(name);
        } else if semantic == TILE_BOUNDING_BOX {
            record.tile_semantic.get_or_insert(TILE_BOUNDING_BOX);
            record.bounding_box = Some(interpreted(id, property, true));
        } else if semantic == TILE_GEOMETRIC_ERROR {
            record.tile_semantic.get_or_insert(TILE_GEOMETRIC_ERROR);
            record.geometric_error = Some(interpreted(id, property, false));
        }
    }
    record
}

/// TILE_BOUNDING_BOX: SCALAR FLOAT32|FLOAT64, `array` true, `count` 12, not
/// normalized. TILE_GEOMETRIC_ERROR: SCALAR FLOAT32|FLOAT64, not an array.
fn interpreted(
    id: &str,
    property: &Map<String, Value>,
    bounding_box: bool,
) -> Result<Interpreted, Defect> {
    let flag = |key: &str| match property.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(Defect::Invalid(format!(
            "property {id:?} {key} must be a boolean"
        ))),
    };
    let float = match property.get("componentType").and_then(Value::as_str) {
        Some("FLOAT32") => Some(Float::F32),
        Some("FLOAT64") => Some(Float::F64),
        _ => None,
    };
    let array = flag("array")?;
    let count = property.get("count");
    let shape = property.get("type").and_then(Value::as_str) == Some("SCALAR")
        && float.is_some()
        && !flag("normalized")?
        && if bounding_box {
            array && count.and_then(Value::as_u64) == Some(12)
        } else {
            !array && count.is_none()
        };
    if !shape {
        return Err(Defect::Invalid(format!(
            "interpreted property {id:?} has an unsupported shape"
        )));
    }
    if let Some(key) = ["offset", "scale", "noData", "default"]
        .into_iter()
        .find(|key| property.contains_key(*key))
    {
        return Err(Defect::Unsupported(format!(
            "{key} on interpreted property {id:?}"
        )));
    }
    Ok(Interpreted {
        id: id.to_owned(),
        float: float.unwrap(),
        required: flag("required")?,
    })
}

/// An interpreted column of the `tileMetadata` table: values read in place.
#[derive(Clone, Copy, Debug)]
pub(super) struct Column {
    pub(super) view: usize,
    pub(super) float: Float,
}

#[derive(Debug, Default)]
pub(super) struct TileColumns {
    pub(super) bounding_box: Option<Column>,
    pub(super) geometric_error: Option<Column>,
}

/// One property table of `propertyTables`, as a typed reference.
fn table<'a>(
    tables: Option<&'a Value>,
    index: &Value,
    label: &str,
) -> Result<&'a Map<String, Value>, FormatError> {
    let index = uint(Some(index), label)?;
    tables
        .and_then(Value::as_array)
        .and_then(|t| usize::try_from(index).ok().and_then(|i| t.get(i)))
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(format!("{label} names no property table")))
}

/// An admitted table's class record and its own columns, if any.
type AdmittedTable<'a> = (Rc<ClassRecord>, Option<&'a Map<String, Value>>);

/// Shared table admission: class, its classification for `use_`, `count`
/// against the memoized popcount, then the table's own columns, each a
/// reference to an existing validated view.
fn admit_table<'a>(
    schema: Option<&super::ImplicitSchema>,
    table: &'a Map<String, Value>,
    use_: Use,
    popcount: u64,
    storage: &Storage,
) -> Result<AdmittedTable<'a>, FormatError> {
    let class = table
        .get("class")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("property table class must be a string"))?;
    let record = schema
        .ok_or_else(|| invalid(format!("unknown metadata class {class:?}")))?
        .admit(class, use_)?;
    let count = uint(table.get("count"), "property table count")?;
    // Schema `minimum: 1`: an empty slot cannot carry a table (DC-8).
    if count == 0 {
        return Err(invalid("property table count must be at least 1"));
    }
    if count != popcount {
        return Err(invalid(format!(
            "property table count {count} differs from availability popcount {popcount}"
        )));
    }
    let properties = match table.get("properties") {
        None => None,
        Some(Value::Object(properties)) => Some(properties),
        Some(_) => return Err(invalid("property table properties must be an object")),
    };
    for (id, column) in properties.into_iter().flatten() {
        let column = column
            .as_object()
            .ok_or_else(|| invalid(format!("property table column {id:?} must be an object")))?;
        if !column.contains_key("values") {
            return Err(invalid(format!(
                "property table column {id:?} lacks values"
            )));
        }
        for key in ["values", "arrayOffsets", "stringOffsets"] {
            if let Some(view) = column.get(key) {
                storage.view_index(view, key)?;
            }
        }
    }
    Ok((record, properties))
}

/// `tileMetadata` admission; interpreted values are read in place and checked.
pub(super) fn tile_table(
    schema: Option<&super::ImplicitSchema>,
    tables: Option<&Value>,
    index: &Value,
    popcount: u64,
    storage: &Storage,
) -> Result<TileColumns, FormatError> {
    let table = table(tables, index, "tileMetadata")?;
    let (record, properties) = admit_table(schema, table, Use::Tile, popcount, storage)?;
    let mut columns = TileColumns::default();
    for (property, lanes, slot) in [
        (&record.bounding_box, 12usize, &mut columns.bounding_box),
        (&record.geometric_error, 1, &mut columns.geometric_error),
    ] {
        let Some(Ok(property)) = property else {
            continue;
        };
        let Some(column) = properties.and_then(|p| p.get(&property.id)) else {
            if property.required {
                return Err(invalid(format!(
                    "required interpreted property {:?} is missing from tileMetadata",
                    property.id
                )));
            }
            continue;
        };
        if let Some(key) = ["offset", "scale"]
            .into_iter()
            .find(|k| column.get(*k).is_some())
        {
            return Err(unsupported(format!(
                "{key} on interpreted property {:?}",
                property.id
            )));
        }
        let view = storage.view_index(&column["values"], "values")?;
        let size = property.float.size();
        let expected = u128::from(popcount) * (lanes * size) as u128;
        let bytes = storage.view(view);
        if bytes.len() as u128 != expected {
            return Err(invalid(format!(
                "interpreted property {:?} values have the wrong byteLength",
                property.id
            )));
        }
        for (row, values) in bytes.chunks_exact(lanes * size).enumerate() {
            let mut lane = values.chunks_exact(size).map(|v| property.float.read(v));
            if lanes == 1 {
                let error = lane.next().unwrap_or(f64::NAN);
                if !error.is_finite() || error < 0. {
                    return Err(invalid(format!(
                        "TILE_GEOMETRIC_ERROR row {row} must be finite and nonnegative"
                    )));
                }
            } else if !lane.all(f64::is_finite) {
                return Err(invalid(format!(
                    "TILE_BOUNDING_BOX row {row} must be finite"
                )));
            }
        }
        *slot = Some(Column {
            view,
            float: property.float,
        });
    }
    Ok(columns)
}

/// One `contentMetadata` entry (slot `k`): counted and type-checked only.
pub(super) fn content_table(
    schema: Option<&super::ImplicitSchema>,
    tables: Option<&Value>,
    index: &Value,
    popcount: u64,
    storage: &Storage,
) -> Result<(), FormatError> {
    let table = table(tables, index, "contentMetadata")?;
    admit_table(schema, table, Use::Content, popcount, storage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{
        admit_root, Cell, EffectiveError, EffectiveVolume, ImplicitSchema, Scheme, VisitedTile,
    };
    use super::{FormatError, CLASS_SCANS};
    use serde_json::{json, Value};

    const ROOT: &str = "s/0/0/0.subtree";

    fn bounding_box() -> Value {
        json!({"type": "SCALAR", "componentType": "FLOAT64", "array": true, "count": 12,
               "semantic": "TILE_BOUNDING_BOX"})
    }
    fn geometric_error() -> Value {
        json!({"type": "SCALAR", "componentType": "FLOAT64", "semantic": "TILE_GEOMETRIC_ERROR"})
    }
    fn class(properties: Value) -> Value {
        json!({"properties": properties})
    }
    fn tile_class() -> Value {
        class(json!({"box": bounding_box(), "err": geometric_error()}))
    }
    fn boxes(r: usize) -> [f64; 12] {
        std::array::from_fn(|lane| (r * 100 + lane) as f64)
    }
    const ERRORS: [f64; 3] = [10., 5., 4.];

    /// QUADTREE `L = 2`, tile bits {0, 2, 3}: ranks 0, 1, 2 in walk order.
    fn rows(class: &str, ids: (&str, &str), float32: bool) -> Sub {
        let mut sub = Sub::new()
            .tile(Av::Bits(bits(5, &[0, 2, 3])))
            .child(Av::Const(0));
        let encode = |values: &mut Vec<u8>, v: f64| {
            if float32 {
                values.extend((v as f32).to_le_bytes())
            } else {
                values.extend(v.to_le_bytes())
            }
        };
        let mut box_values = Vec::new();
        let mut error_values = Vec::new();
        for (r, error) in ERRORS.iter().enumerate() {
            boxes(r).iter().for_each(|v| encode(&mut box_values, *v));
            encode(&mut error_values, *error);
        }
        let b = sub.view(&box_values);
        let e = sub.view(&error_values);
        sub.set(
            "propertyTables",
            json!([{"class": class, "count": 3, "properties": {
                ids.0: {"values": b}, ids.1: {"values": e}
            }}]),
        )
        .set("tileMetadata", json!(0))
    }
    fn table(sub: &mut Sub) -> &mut Value {
        &mut sub.json["propertyTables"][0]
    }
    fn run_with(schema: &ImplicitSchema, sub: &Sub) -> Result<Vec<VisitedTile>, FormatError> {
        run_tile(&root_tile("QUADTREE", 2, 2, &[]), schema, sub)
    }
    fn run_tile(
        tile: &Value,
        schema: &ImplicitSchema,
        sub: &Sub,
    ) -> Result<Vec<VisitedTile>, FormatError> {
        let root = admit_root(tile, "tileset.json", Some(schema)).map_err(format_only)?;
        let mut archive = Archive::default().with(ROOT, sub.build());
        format(walk_with(&root, &mut archive, &mut limits()))
    }
    fn schema(name: &str, class: Value) -> ImplicitSchema {
        ImplicitSchema::new(json!({"classes": {name: class}}))
    }

    #[test]
    fn rows_follow_strict_rank_by_semantic() {
        // B-1 and the strict rank: each tile reads row popcount(tile[0..i)).
        let tiles = run_with(
            &schema("t", tile_class()),
            &rows("t", ("box", "err"), false),
        )
        .unwrap();
        assert_eq!(
            coords(&tiles),
            vec![(0, 0, 0, 0), (1, 1, 0, 0), (1, 0, 1, 0)]
        );
        for (r, tile) in tiles.iter().enumerate() {
            assert_eq!(tile.volume, EffectiveVolume::Row(boxes(r)));
            assert_eq!(tile.error, EffectiveError::Row(ERRORS[r]));
        }
        // B-3: renamed class and properties keep the facts (semantic, not name).
        let renamed = class(json!({"q": bounding_box(), "rustyTile": {"type": "STRING"},
                                   "r": geometric_error()}));
        assert_eq!(
            run_with(&schema("other", renamed), &rows("other", ("q", "r"), false)).unwrap(),
            tiles
        );
        // B-6: FLOAT32 rows widen exactly.
        let mut float32 = tile_class();
        float32["properties"]["box"]["componentType"] = json!("FLOAT32");
        float32["properties"]["err"]["componentType"] = json!("FLOAT32");
        let widened = run_with(
            &schema("t", float32.clone()),
            &rows("t", ("box", "err"), true),
        )
        .unwrap();
        assert_eq!(widened, tiles);
        // Read with a FLOAT64 stride, the same columns have the wrong length.
        assert_invalid(
            run_with(&schema("t", tile_class()), &rows("t", ("box", "err"), true)),
            "wrong byteLength",
        );
        let mut fraction = rows("t", ("box", "err"), true);
        let e = fraction.view(
            &[
                0.1f32.to_le_bytes(),
                0.2f32.to_le_bytes(),
                0.3f32.to_le_bytes(),
            ]
            .concat(),
        );
        table(&mut fraction)["properties"]["err"]["values"] = json!(e);
        let tiles = run_with(&schema("t", float32), &fraction).unwrap();
        assert_eq!(tiles[1].error, EffectiveError::Row(f64::from(0.2f32)));
    }

    #[test]
    fn missing_semantics_and_optional_columns_fall_back_to_derived() {
        let root_box = [0., 0., 0., 16., 0., 0., 0., 16., 0., 0., 0., 16.];
        let derived = |coords: [u32; 3]| {
            EffectiveVolume::Derived(Cell {
                root: root_box,
                level: 1,
                coords,
                scheme: Scheme::Quadtree,
            })
        };
        // B-4: the semantics removed; the columns are opaque.
        let plain = class(json!({"box": {"type": "SCALAR", "componentType": "FLOAT64",
                                         "array": true, "count": 12},
                                 "err": {"type": "SCALAR", "componentType": "FLOAT64"}}));
        let tiles = run_with(&schema("t", plain), &rows("t", ("box", "err"), false)).unwrap();
        assert_eq!(
            tiles.iter().map(|t| t.volume.clone()).collect::<Vec<_>>(),
            vec![
                EffectiveVolume::DerivedRoot,
                derived([1, 0, 0]),
                derived([0, 1, 0])
            ]
        );
        assert_eq!(
            tiles.iter().map(|t| t.error).collect::<Vec<_>>(),
            vec![
                EffectiveError::Derived(64.),
                EffectiveError::Derived(32.),
                EffectiveError::Derived(32.)
            ]
        );
        // B-12: an optional interpreted column omitted from the table.
        let mut sub = rows("t", ("box", "err"), false);
        table(&mut sub)["properties"]
            .as_object_mut()
            .unwrap()
            .remove("box");
        let tiles = run_with(&schema("t", tile_class()), &sub).unwrap();
        assert_eq!(tiles[0].volume, EffectiveVolume::DerivedRoot);
        assert_eq!(tiles[2].volume, derived([0, 1, 0]));
        assert_eq!(tiles[2].error, EffectiveError::Row(4.));
        // B-11: a required one omitted.
        let mut required = tile_class();
        required["properties"]["box"]["required"] = json!(true);
        assert_invalid(
            run_with(&schema("t", required), &sub),
            "required interpreted property \"box\"",
        );
        // No tileMetadata at all: everything derived.
        let tiles = run_with(
            &schema("t", tile_class()),
            &Sub::new().tile(Av::Const(1)).child(Av::Const(0)),
        )
        .unwrap();
        assert_eq!(tiles.len(), 5);
        assert_eq!(tiles[4].error, EffectiveError::Derived(32.));
    }

    #[test]
    fn interpreted_shapes_and_modifiers() {
        let sub = rows("t", ("box", "err"), false);
        let with = |path: (&str, &str), value: Value| {
            let mut class = tile_class();
            class["properties"][path.0][path.1] = value;
            run_with(&schema("t", class), &sub)
        };
        // B-5.
        let mut duplicate = tile_class();
        duplicate["properties"]["again"] = geometric_error();
        assert_invalid(
            run_with(&schema("t", duplicate), &sub),
            "duplicated semantic",
        );
        // B-7 and the other modifiers, class level.
        for key in ["offset", "scale", "noData", "default"] {
            assert_unsupported(
                with(("box", key), json!(1)),
                &format!("{key} on interpreted"),
            );
            assert_unsupported(
                with(("err", key), json!(1)),
                &format!("{key} on interpreted"),
            );
        }
        // Table-level offset or scale.
        for key in ["offset", "scale"] {
            let mut scaled = sub.clone();
            table(&mut scaled)["properties"]["err"][key] = json!(2);
            assert_unsupported(
                run_with(&schema("t", tile_class()), &scaled),
                &format!("{key} on interpreted"),
            );
        }
        for (path, value) in [
            (("box", "count"), json!(11)),
            (("box", "array"), json!(false)),
            (("box", "normalized"), json!(true)),
            (("box", "type"), json!("VEC3")),
            (("err", "array"), json!(true)),
            (("err", "count"), json!(2)),
            (("err", "componentType"), json!("INT32")),
        ] {
            assert_invalid(with(path, value), "unsupported shape");
        }
        assert_invalid(with(("err", "required"), json!(1)), "must be a boolean");
    }

    #[test]
    fn row_counts_and_values() {
        let s = schema("t", tile_class());
        let mut sub = rows("t", ("box", "err"), false);
        for count in [0, 2, 4] {
            table(&mut sub)["count"] = json!(count);
            let needle = if count == 0 {
                "at least 1"
            } else {
                "differs from availability popcount"
            };
            assert_invalid(run_with(&s, &sub), needle);
        }
        // B-9: error −1 or NaN; a non-finite box lane.
        for (column, row) in [
            ("err", [10f64, -1., 4.].map(f64::to_le_bytes).concat()),
            ("err", [10f64, f64::NAN, 4.].map(f64::to_le_bytes).concat()),
            ("box", {
                let mut values: Vec<u8> =
                    (0..36).flat_map(|v| f64::from(v).to_le_bytes()).collect();
                values[8 * 30..8 * 31].copy_from_slice(&f64::INFINITY.to_le_bytes());
                values
            }),
        ] {
            let mut bad = rows("t", ("box", "err"), false);
            let v = bad.view(&row);
            table(&mut bad)["properties"][column]["values"] = json!(v);
            assert_invalid(run_with(&s, &bad), "must be finite");
        }
    }

    #[test]
    fn unsupported_semantics_in_walked_tables() {
        let sub = rows("t", ("box", "err"), false);
        for semantic in [
            "TILE_BOUNDING_SPHERE",
            "TILE_TRANSFORM",
            "CONTENT_URI",
            "TILE_MAXIMUM_HEIGHT",
        ] {
            let mut class = tile_class();
            class["properties"]["extra"] = json!({"type": "STRING", "semantic": semantic});
            assert_unsupported(run_with(&schema("t", class), &sub), semantic);
        }
        // An unused class never matters.
        let s = ImplicitSchema::new(json!({"classes": {"t": tile_class(), "unused":
            class(json!({"u": {"type": "STRING", "semantic": "CONTENT_URI"}}))}}));
        assert!(run_with(&s, &sub).is_ok());
    }

    fn content_root(slots: usize) -> Value {
        let templates = ["a/{level}/{x}/{y}.glb", "b/{level}/{x}/{y}.glb"];
        root_tile("QUADTREE", 2, 2, &templates[..slots])
    }
    /// Two slots: slot 0 on bits {0, 2}, slot 1 per `second`.
    fn content_sub(second: Av, tables: Value, metadata: Value) -> Sub {
        let mut sub = Sub::new().tile(Av::Const(1)).child(Av::Const(0));
        let values = sub.view(&[0; 4]);
        let tables = tables
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t["properties"] = json!({"p": {"values": values}});
                t
            })
            .collect();
        sub.content(&[Av::Bits(bits(5, &[0, 2])), second])
            .set("propertyTables", Value::Array(tables))
            .set("contentMetadata", metadata)
    }

    #[test]
    fn content_metadata_tables() {
        let opaque = schema("c", class(json!({"p": {"type": "UINT8"}})));
        let tables = json!([{"class": "c", "count": 2}, {"class": "c", "count": 5}]);
        let ok = content_sub(Av::Const(1), tables.clone(), json!([0, 1]));
        assert!(run_tile(&content_root(2), &opaque, &ok).is_ok());
        // One entry per template.
        let short = content_sub(Av::Const(1), tables, json!([0]));
        assert_invalid(
            run_tile(&content_root(2), &opaque, &short),
            "one entry per content template",
        );
        // D-10: slot 1 has popcount 0, so it cannot carry a table at all.
        for (count, needle) in [(0, "at least 1"), (1, "differs from availability popcount")] {
            let tables = json!([{"class": "c", "count": 2}, {"class": "c", "count": count}]);
            let sub = content_sub(Av::Const(0), tables, json!([0, 1]));
            assert_invalid(run_tile(&content_root(2), &opaque, &sub), needle);
        }
        // C-12, C-13 and tile semantics in a content class.
        for semantic in [
            "CONTENT_URI",
            "CONTENT_GROUP_ID",
            "CONTENT_BOUNDING_BOX",
            "TILE_GEOMETRIC_ERROR",
        ] {
            let s = schema(
                "c",
                class(
                    json!({"p": {"type": "UINT8"}, "u": {"type": "STRING", "semantic": semantic}}),
                ),
            );
            assert_unsupported(run_tile(&content_root(2), &s, &ok), semantic);
        }
        // CONTENT_URI values are never read, even when they name JSON.
        let s = schema(
            "c",
            class(json!({"p": {"type": "STRING", "semantic": "CONTENT_URI"}})),
        );
        let mut naming = ok.clone();
        let json_name = naming.view(b"x.json");
        naming.json["propertyTables"][0]["properties"]["p"]["values"] = json!(json_name);
        assert_unsupported(run_tile(&content_root(2), &s, &naming), "CONTENT_URI");
    }

    #[test]
    fn table_structure() {
        let s = schema("t", tile_class());
        let sub = rows("t", ("box", "err"), false);
        let mutate = |f: &dyn Fn(&mut Value)| {
            let mut sub = sub.clone();
            f(table(&mut sub));
            run_with(&s, &sub)
        };
        assert_invalid(
            mutate(&|t| t["class"] = json!("x")),
            "unknown metadata class \"x\"",
        );
        assert_invalid(mutate(&|t| t["class"] = json!(1)), "class must be a string");
        assert_invalid(
            mutate(&|t| t["properties"] = json!([])),
            "must be an object",
        );
        assert_invalid(
            mutate(&|t| t["properties"]["other"] = json!({"values": 9})),
            "names no bufferView",
        );
        assert_invalid(
            mutate(&|t| t["properties"]["other"] = json!({"values": 0, "stringOffsets": 9})),
            "names no bufferView",
        );
        assert_invalid(
            mutate(&|t| t["properties"]["other"] = json!({})),
            "lacks values",
        );
        let root = admit_root(&root_tile("QUADTREE", 2, 2, &[]), "tileset.json", None).unwrap();
        let mut archive = Archive::default().with(ROOT, sub.build());
        assert_invalid(
            format(walk_with(&root, &mut archive, &mut limits())),
            "unknown metadata class",
        );
        // A malformed class is reported on use.
        assert_invalid(
            run_with(&schema("t", json!({"properties": []})), &sub),
            "must be objects",
        );
    }

    #[test]
    fn classes_are_classified_once_per_schema_document() {
        // R7c at small scale: QUADTREE L = 1, A = 4, one tile per subtree,
        // child Const1 at levels 0–2: 1 + 4 + 16 + 64 = 85 subtrees, each with
        // a one-row table of a 200-property class. Two roots share the schema.
        let mut properties = serde_json::Map::new();
        for i in 0..200 {
            properties.insert(format!("p{i}"), json!({"type": "BOOLEAN"}));
        }
        let s = schema("big", json!({"properties": properties}));
        let member = |child: u64| {
            let mut sub = Sub::new().tile(Av::Const(1)).child(Av::Const(child));
            let v = sub.view(&[1]);
            sub.set(
                "propertyTables",
                json!([{"class": "big", "count": 1, "properties": {"p7": {"values": v}}}]),
            )
            .set("tileMetadata", json!(0))
            .build()
        };
        let mut archive = Archive::default();
        for level in 0..4u32 {
            for x in 0..1u32 << level {
                for y in 0..1u32 << level {
                    let child = u64::from(level < 3);
                    for prefix in ["s", "u"] {
                        archive
                            .members
                            .insert(format!("{prefix}/{level}/{x}/{y}.subtree"), member(child));
                    }
                }
            }
        }
        CLASS_SCANS.with(|n| n.set(0));
        let mut limits = limits();
        let mut tile = root_tile("QUADTREE", 1, 4, &[]);
        for prefix in ["s", "u"] {
            tile["implicitTiling"]["subtrees"]["uri"] =
                json!(format!("{prefix}/{{level}}/{{x}}/{{y}}.subtree"));
            let root = admit_root(&tile, "tileset.json", Some(&s)).unwrap();
            assert_eq!(
                walk_with(&root, &mut archive, &mut limits).unwrap().len(),
                85
            );
        }
        assert_eq!(CLASS_SCANS.with(|n| n.get()), 1);
    }
}
