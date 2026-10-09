//! GeoJSON has one bounded semantic reader in every build. Other native
//! formats retain their native source reader; geometry kernels are independent.
use super::*;
pub(super) enum Reader {
    Portable(Box<source_portable::Reader>),
    #[cfg(feature = "native-geospatial")]
    Native(Box<source_native::Reader>),
}
impl Reader {
    pub fn new(
        input: &Path,
        options: &VectorOptions,
        frame: Option<Frame>,
        scratch: &Path,
    ) -> Result<Self, Error> {
        #[cfg(feature = "native-geospatial")]
        if !input
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("geojson") || e.eq_ignore_ascii_case("json"))
        {
            let mut reader = source_native::Reader::new(input, options, frame, scratch)?;
            if matches!(reader.driver.as_str(), "GeoJSON" | "GeoJSONSeq") {
                reader.finish()?;
                return Err(Error::Environment("GeoJSON requires .geojson or .json input through the shared bounded reader; GeoJSON sequences are unsupported".into()));
            }
            return Ok(Self::Native(Box::new(reader)));
        }
        source_portable::Reader::new(input, options, frame, scratch)
            .map(Box::new)
            .map(Self::Portable)
    }
    pub fn read(
        &mut self,
        options: &VectorOptions,
        accept: impl FnMut(Feature) -> FeatureResult<()>,
        report: impl FnMut(Value, bool) -> Result<(), Error>,
    ) -> Result<(), Error> {
        match self {
            Self::Portable(reader) => reader.read(options, accept, report),
            #[cfg(feature = "native-geospatial")]
            Self::Native(reader) => reader.read(options, accept, report),
        }
    }
    pub fn driver(&self) -> &str {
        match self {
            Self::Portable(r) => &r.driver,
            #[cfg(feature = "native-geospatial")]
            Self::Native(r) => &r.driver,
        }
    }
    pub fn schemas(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Portable(r) => &r.schemas,
            #[cfg(feature = "native-geospatial")]
            Self::Native(r) => &r.schemas,
        }
    }
    pub fn layer_reports(&self) -> &[Value] {
        match self {
            Self::Portable(r) => &r.layer_reports,
            #[cfg(feature = "native-geospatial")]
            Self::Native(r) => &r.layer_reports,
        }
    }
    pub fn frame(&self) -> Option<&Frame> {
        match self {
            Self::Portable(r) => r.frame.as_ref(),
            #[cfg(feature = "native-geospatial")]
            Self::Native(r) => r.frame.as_ref(),
        }
    }
    pub fn frame_mut(&mut self) -> &mut Option<Frame> {
        match self {
            Self::Portable(r) => &mut r.frame,
            #[cfg(feature = "native-geospatial")]
            Self::Native(r) => &mut r.frame,
        }
    }
    pub fn without_geometry(&self) -> usize {
        match self {
            Self::Portable(r) => r.without_geometry,
            #[cfg(feature = "native-geospatial")]
            Self::Native(r) => r.without_geometry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &Path, features: Value) {
        std::fs::write(
            path,
            serde_json::to_vec(&json!({"type":"FeatureCollection","features":features})).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn malformed_geojson_properties_reject_before_null_geometry_disposition() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.geojson");
        fixture(
            &input,
            json!([
                {"type":"Feature","id":"bad-point","properties":7,"geometry":{"type":"Point","coordinates":[100,200,300]}},
                {"type":"Feature","id":"bad-null","properties":"wrong","geometry":null},
                {"type":"Feature","id":"accepted","properties":{"name":"kept"},"geometry":{"type":"Point","coordinates":[1,2,3]}}
            ]),
        );
        let options = VectorOptions {
            source_crs: Some("local".into()),
            skip_invalid: true,
            ..VectorOptions::default()
        };
        let mut reader = Reader::new(&input, &options, None, work.path()).unwrap();
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        reader
            .read(
                &options,
                |feature| {
                    accepted.push(feature.source_id().clone());
                    Ok(())
                },
                |value, invalid| {
                    rejected.push((value, invalid));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(accepted, vec![json!("\"accepted\"")]);
        assert_eq!(rejected.len(), 2);
        assert!(rejected.iter().all(|(value, invalid)| *invalid
            && value["reason"] == "GeoJSON feature properties must be an object or null"));
        assert_eq!(reader.without_geometry(), 0);
        assert_eq!(reader.frame().unwrap().anchor, [1., 2., 3.]);
        assert_eq!(
            reader.schemas().get("name").map(String::as_str),
            Some("string")
        );
    }
    #[cfg(feature = "native-geospatial")]
    #[test]
    fn public_admission_refuses_renamed_geojson_and_sequences_in_both_policies() {
        use crate::vector::{vector_to_archive, VectorRequest};
        for sequence in [false, true] {
            for skip_invalid in [false, true] {
                let work = tempfile::tempdir().unwrap();
                let input = work.path().join(if sequence {
                    "source.geojsonl"
                } else {
                    "source.dat"
                });
                let output = work.path().join("output.3tz");
                let feature = json!({"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2,3,4]}});
                let bytes = if sequence {
                    format!("\x1e{}\n", serde_json::to_string(&feature).unwrap())
                } else {
                    serde_json::to_string(&json!({"type":"FeatureCollection","features":[feature]}))
                        .unwrap()
                };
                std::fs::write(&input, &bytes).unwrap();
                std::fs::write(&output, b"existing archive").unwrap();
                let options = VectorOptions {
                    source_crs: Some("local".into()),
                    skip_invalid,
                    ..VectorOptions::default()
                };
                let error = vector_to_archive(
                    VectorRequest::new(&input, &output, options)
                        .with_policy(crate::OutputPolicy::Replace),
                    &crate::RunControl::default(),
                )
                .unwrap_err();
                assert_eq!(error.error.kind(), crate::JobErrorKind::Unsupported);
                assert_eq!(std::fs::read(&output).unwrap(), b"existing archive");
                assert_eq!(std::fs::read_to_string(&input).unwrap(), bytes);
                assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 2);
            }
        }
    }
}
