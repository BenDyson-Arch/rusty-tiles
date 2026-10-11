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
