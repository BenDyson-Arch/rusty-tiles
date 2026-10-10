//! Exact scalar metadata policies shared by vector readers.
use super::pipeline::acceptance::{FeatureFailure, FeatureResult};
use super::VectorOptions;
use crate::Error;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn keep(options: &VectorOptions, name: &str) -> bool {
    (options.fields.is_empty() || options.fields.iter().any(|f| f == name))
        && !options.drop_fields.iter().any(|f| f == name)
}

/// Reconcile selected source fields across rows and layers. Integer/real
/// promotion changes the schema only; the original JSON numbers stay exact.
pub(super) fn register(
    schemas: &mut BTreeMap<String, String>,
    name: &str,
    kind: &str,
) -> FeatureResult<()> {
    let kind = match schemas.get(name).map(String::as_str) {
        Some(previous) if previous != kind => {
            if matches!(previous, "integer" | "real") && matches!(kind, "integer" | "real") {
                "real"
            } else {
                return Err(FeatureFailure::reject(format!(
                    "incompatible scalar schemas for {name:?}; convert these layers separately"
                )));
            }
        }
        _ => kind,
    };
    schemas.insert(name.into(), kind.into());
    Ok(())
}

/// The feature's JSON ID is stored as a string, including string quotes and a
/// literal `null`. Missing GeoJSON IDs fall back to the reader's numeric FID.
pub(super) fn source_id(native: Option<&Value>, fid: i64) -> Result<String, Error> {
    let id = native
        .and_then(|v| v.get("id"))
        .cloned()
        .unwrap_or(json!(fid));
    Ok(serde_json::to_string(&id)?)
}

/// Read original GeoJSON values instead of a driver's potentially lossy field
/// coercions. Selection happens before validating unsupported complex values.
/// Explicit JSON list mode encodes arrays (including heterogeneous arrays) as
/// strings while preserving order, exact integers and null values.
pub(super) fn geojson_properties(
    native: &Value,
    options: &VectorOptions,
    schemas: &mut BTreeMap<String, String>,
    json_fields: &mut BTreeSet<String>,
) -> FeatureResult<BTreeMap<String, Value>> {
    let mut properties = BTreeMap::new();
    if let Some(props) = native["properties"].as_object() {
        for (key, value) in props {
            if !keep(options, key) {
                continue;
            }
            if matches!(key.as_str(), "_source_id" | "_source_layer") {
                return Err(FeatureFailure::reject(format!(
                    "reserved source property: {key}"
                )));
            }
            let value = if value.is_array() && options.list_fields == "json" {
                json_fields.insert(key.clone());
                json!(serde_json::to_string(value)?)
            } else {
                value.clone()
            };
            if !value.is_null() {
                let kind = if value.is_boolean() {
                    "boolean"
                } else if value.is_i64() || value.is_u64() {
                    "integer"
                } else if value.is_f64() {
                    "real"
                } else if value.is_string() {
                    "string"
                } else {
                    return Err(FeatureFailure::reject(format!(
                        "unsupported complex property: {key}"
                    )));
                };
                register(schemas, key, kind)?;
            }
            properties.insert(key.clone(), value);
        }
    }
    Ok(properties)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(
        properties: Value,
        options: &VectorOptions,
        schemas: &mut BTreeMap<String, String>,
    ) -> Result<(BTreeMap<String, Value>, BTreeSet<String>), Error> {
        let mut json_fields = BTreeSet::new();
        let properties = geojson_properties(
            &json!({"properties":properties}),
            options,
            schemas,
            &mut json_fields,
        )?;
        Ok((properties, json_fields))
    }

    #[test]
    fn exact_scalars_nulls_and_missing_properties_are_preserved() {
        let source: Value = serde_json::from_str(
            r#"{"signed":-9223372036854775808,"unsigned":18446744073709551615,"large":1152921504606846979,"boolean":true,"real":1.25,"text":"é","null":null}"#,
        )
        .unwrap();
        let mut schemas = BTreeMap::new();
        let (properties, json_fields) =
            read(source.clone(), &VectorOptions::default(), &mut schemas).unwrap();
        assert_eq!(serde_json::to_value(&properties).unwrap(), source);
        assert!(json_fields.is_empty());
        assert_eq!(properties["signed"].as_i64(), Some(i64::MIN));
        assert_eq!(properties["unsigned"].as_u64(), Some(u64::MAX));
        assert_eq!(schemas["large"], "integer");
        assert_eq!(schemas["boolean"], "boolean");
        assert_eq!(schemas["real"], "real");
        assert_eq!(schemas["text"], "string");
        assert!(!schemas.contains_key("null"));
        let (next, _) = read(
            json!({"null":null}),
            &VectorOptions::default(),
            &mut schemas,
        )
        .unwrap();
        assert_eq!(next.len(), 1);
        assert!(!next.contains_key("large"));
    }

    #[test]
    fn source_ids_keep_json_representation_and_distinguish_absent_from_null() {
        for (native, expected) in [
            (json!({}), "-1"),
            (json!({"id":null}), "null"),
            (json!({"id":"quoted \"id\""}), r#""quoted \"id\"""#),
            (json!({"id":u64::MAX}), "18446744073709551615"),
        ] {
            assert_eq!(source_id(Some(&native), -1).unwrap(), expected);
        }
        assert_eq!(source_id(None, 42).unwrap(), "42");
    }

    #[test]
    fn json_lists_keep_heterogeneous_values_order_and_exact_integers() {
        let options = VectorOptions {
            list_fields: "json".into(),
            ..VectorOptions::default()
        };
        let list = json!([u64::MAX, -1152921504606846979i64, null, true, "a", {"nested":[1,2]}]);
        let mut schemas = BTreeMap::new();
        let (properties, fields) = read(
            json!({"list":list, "empty":[], "null":null}),
            &options,
            &mut schemas,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(properties["list"].as_str().unwrap()).unwrap(),
            list
        );
        assert_eq!(properties["empty"], "[]");
        assert!(properties["null"].is_null());
        assert_eq!(fields, BTreeSet::from(["empty".into(), "list".into()]));
        assert_eq!(schemas["list"], "string");
        assert!(read(
            json!({"list":[1,2]}),
            &VectorOptions::default(),
            &mut BTreeMap::new()
        )
        .is_err());
        assert!(read(json!({"object":{}}), &options, &mut BTreeMap::new()).is_err());
    }

    #[test]
    fn selection_precedes_complex_and_reserved_property_validation() {
        let source = json!({"name":"keep", "object":{}, "_source_id":"reserved"});
        for options in [
            VectorOptions {
                fields: vec!["name".into()],
                ..VectorOptions::default()
            },
            VectorOptions {
                drop_fields: vec!["object".into(), "_source_id".into()],
                ..VectorOptions::default()
            },
        ] {
            let (properties, _) = read(source.clone(), &options, &mut BTreeMap::new()).unwrap();
            assert_eq!(properties, BTreeMap::from([("name".into(), json!("keep"))]));
        }
        for name in ["_source_id", "_source_layer"] {
            assert!(read(
                json!({name:null}),
                &VectorOptions::default(),
                &mut BTreeMap::new()
            )
            .unwrap_err()
            .to_string()
            .contains("reserved source property"));
        }
    }

    #[test]
    fn row_and_layer_schemas_promote_numbers_and_refuse_incompatible_scalars() {
        for kinds in [["integer", "real", "integer"], ["real", "integer", "real"]] {
            let mut schemas = BTreeMap::new();
            for kind in kinds {
                register(&mut schemas, "same", kind).unwrap();
            }
            assert_eq!(schemas["same"], "real");
            for incompatible in ["boolean", "string"] {
                let error = register(&mut schemas, "same", incompatible).unwrap_err();
                assert!(error
                    .to_string()
                    .contains("convert these layers separately"));
                assert_eq!(schemas["same"], "real");
            }
        }
        let mut schemas = BTreeMap::new();
        register(&mut schemas, "flag", "boolean").unwrap();
        assert!(register(&mut schemas, "flag", "integer").is_err());
        assert_eq!(schemas["flag"], "boolean");
    }
}
