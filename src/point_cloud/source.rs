//! LAS record layout and Extra Bytes decoding. Numeric source bytes are copied
//! directly, avoiding lossy round trips through las::Point or floating integers.
use crate::Error;

// Scratch records: relative XYZ, original source index, scaled source XYZ, then
// the complete decompressed LAS record. All fields use explicit little endian.
pub(super) const RAW: usize = 56;

#[derive(Clone, Copy, Debug)]
pub(super) enum Scalar {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

impl Scalar {
    pub fn width(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
        }
    }

    pub fn component(self) -> &'static str {
        match self {
            Self::U8 => "UINT8",
            Self::I8 => "INT8",
            Self::U16 => "UINT16",
            Self::I16 => "INT16",
            Self::U32 => "UINT32",
            Self::I32 => "INT32",
            Self::U64 => "UINT64",
            Self::I64 => "INT64",
            Self::F32 => "FLOAT32",
            Self::F64 => "FLOAT64",
        }
    }

    fn number(self, bytes: &[u8]) -> f64 {
        match self {
            Self::U8 => bytes[0] as f64,
            Self::I8 => bytes[0] as i8 as f64,
            Self::U16 => u16::from_le_bytes(bytes[..2].try_into().unwrap()) as f64,
            Self::I16 => i16::from_le_bytes(bytes[..2].try_into().unwrap()) as f64,
            Self::U32 => u32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
            Self::I32 => i32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
            Self::U64 => u64::from_le_bytes(bytes[..8].try_into().unwrap()) as f64,
            Self::I64 => i64::from_le_bytes(bytes[..8].try_into().unwrap()) as f64,
            Self::F32 => f32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
            Self::F64 => f64_at(bytes, 0),
        }
    }

    fn from_extra(code: u8) -> Result<Self, Error> {
        Ok(match code {
            1 => Self::U8,
            2 => Self::I8,
            3 => Self::U16,
            4 => Self::I16,
            5 => Self::U32,
            6 => Self::I32,
            7 => Self::U64,
            8 => Self::I64,
            9 => Self::F32,
            10 => Self::F64,
            _ => {
                return Err(Error::Data(
                    "unsupported LAS dimension: array or untyped Extra Bytes".into(),
                ))
            }
        })
    }
}

#[derive(Debug)]
enum Decode {
    Raw(usize),
    Bits {
        at: usize,
        shift: u8,
        mask: u8,
    },
    Scaled {
        at: usize,
        input: Scalar,
        scale: f64,
        offset: f64,
    },
}

#[derive(Debug)]
pub(super) struct Dimension {
    pub name: String,
    pub kind: Scalar,
    decode: Decode,
}

impl Dimension {
    pub fn append(&self, row: &[u8], output: &mut Vec<u8>) {
        match self.decode {
            Decode::Raw(at) => output.extend_from_slice(&row[at..at + self.kind.width()]),
            Decode::Bits { at, shift, mask } => output.push((row[at] >> shift) & mask),
            Decode::Scaled {
                at,
                input,
                scale,
                offset,
            } => {
                // Match decoded LAS values: multiplication followed by addition,
                // not fused arithmetic (and not integer truncation).
                output
                    .extend_from_slice(&(input.number(&row[at..]) * scale + offset).to_le_bytes());
            }
        }
    }

    fn validate(&self, raw: &[u8]) -> Result<(), Error> {
        let value = match self.decode {
            Decode::Raw(at) if matches!(self.kind, Scalar::F32 | Scalar::F64) => {
                self.kind.number(&raw[at - RAW..])
            }
            Decode::Scaled {
                at,
                input,
                scale,
                offset,
            } => input.number(&raw[at - RAW..]) * scale + offset,
            _ => return Ok(()),
        };
        if !value.is_finite() {
            return Err(Error::Data(format!("nonfinite dimension: {}", self.name)));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Layout {
    pub dimensions: Vec<Dimension>,
    pub record_len: usize,
    pub rgb: Option<usize>,
}

impl Layout {
    pub fn new(header: &las::Header) -> Result<Self, Error> {
        use Scalar::*;
        let format = header.point_format().to_u8().map_err(las_error)? & 0x3f;
        let base = match format {
            0 => 20,
            1 => 28,
            2 => 26,
            3 => 34,
            6 => 30,
            7 => 36,
            8 => 38,
            4 | 5 | 9 | 10 => {
                return Err(Error::Data(
                    "waveform LAS point formats are unsupported".into(),
                ))
            }
            _ => {
                return Err(Error::Data(format!(
                    "unsupported LAS point format: {format}"
                )))
            }
        };
        let rgb = match format {
            2 => Some(20),
            3 => Some(28),
            7 | 8 => Some(30),
            _ => None,
        };
        let mut result = Self {
            dimensions: Vec::new(),
            record_len: RAW + header.point_format().len() as usize,
            rgb,
        };
        for (name, kind, at) in [
            ("source_index", U64, 24),
            ("source_x", F64, 32),
            ("source_y", F64, 40),
            ("source_z", F64, 48),
        ] {
            result.dimensions.push(Dimension {
                name: name.into(),
                kind,
                decode: Decode::Raw(at),
            });
        }
        for (name, kind, at) in [
            ("X", I32, 0),
            ("Y", I32, 4),
            ("Z", I32, 8),
            ("intensity", U16, 12),
        ] {
            result.raw(name, kind, at);
        }
        let extended = format >= 6;
        result.bits("return_number", 14, 0, if extended { 15 } else { 7 });
        result.bits(
            "number_of_returns",
            14,
            if extended { 4 } else { 3 },
            if extended { 15 } else { 7 },
        );
        if extended {
            for (name, shift) in [
                ("synthetic", 0),
                ("key_point", 1),
                ("withheld", 2),
                ("overlap", 3),
            ] {
                result.bits(name, 15, shift, 1);
            }
            result.bits("scanner_channel", 15, 4, 3);
            result.bits("scan_direction_flag", 15, 6, 1);
            result.bits("edge_of_flight_line", 15, 7, 1);
            result.raw("classification", U8, 16);
            result.raw("user_data", U8, 17);
            result.raw("scan_angle", I16, 18);
            result.raw("point_source_id", U16, 20);
            result.raw("gps_time", F64, 22);
        } else {
            result.bits("scan_direction_flag", 14, 6, 1);
            result.bits("edge_of_flight_line", 14, 7, 1);
            result.bits("classification", 15, 0, 31);
            for (name, shift) in [("synthetic", 5), ("key_point", 6), ("withheld", 7)] {
                result.bits(name, 15, shift, 1);
            }
            result.raw("scan_angle_rank", I8, 16);
            result.raw("user_data", U8, 17);
            result.raw("point_source_id", U16, 18);
            if matches!(format, 1 | 3) {
                result.raw("gps_time", F64, 20);
            }
        }
        if let Some(at) = rgb {
            for (i, name) in ["red", "green", "blue"].into_iter().enumerate() {
                result.raw(name, U16, at + i * 2);
            }
        }
        if format == 8 {
            result.raw("nir", U16, 36);
        }

        // Extra Bytes descriptors are 192-byte LASF_Spec/4 entries. Do not infer
        // a type for undocumented record tails or silently discard them.
        let mut at = base;
        let descriptors: Vec<_> = header
            .all_vlrs()
            .filter(|v| v.user_id == "LASF_Spec" && v.record_id == 4)
            .collect();
        if descriptors.len() > 1 {
            return Err(Error::Data(
                "multiple Extra Bytes VLRs are ambiguous".into(),
            ));
        }
        for vlr in descriptors {
            if !vlr.data.len().is_multiple_of(192) {
                return Err(Error::Data("malformed Extra Bytes VLR".into()));
            }
            for descriptor in vlr.data.as_chunks::<192>().0 {
                let name = text(&descriptor[4..36])?;
                if name.is_empty()
                    || name == "position"
                    || result.dimensions.iter().any(|d| d.name == name)
                {
                    return Err(Error::Data(format!(
                        "reserved or duplicate LAS dimension: {name}"
                    )));
                }
                let input = Scalar::from_extra(descriptor[2])?;
                let width = input.width();
                if RAW + at + width > result.record_len {
                    return Err(Error::Data(
                        "Extra Bytes descriptor exceeds LAS record length".into(),
                    ));
                }
                let flags = descriptor[3];
                if flags & 0xe0 != 0 {
                    return Err(Error::Data("unsupported Extra Bytes option bits".into()));
                }
                if flags & 24 != 0 {
                    let scale = if flags & 8 != 0 {
                        f64_at(descriptor, 112)
                    } else {
                        1.
                    };
                    let offset = if flags & 16 != 0 {
                        f64_at(descriptor, 136)
                    } else {
                        0.
                    };
                    if !scale.is_finite() || !offset.is_finite() {
                        return Err(Error::Data(format!(
                            "nonfinite Extra Bytes scale/offset: {name}"
                        )));
                    }
                    result.dimensions.push(Dimension {
                        name,
                        kind: F64,
                        decode: Decode::Scaled {
                            at: RAW + at,
                            input,
                            scale,
                            offset,
                        },
                    });
                } else {
                    result.raw(&name, input, at);
                }
                at += width;
            }
        }
        if RAW + at != result.record_len {
            return Err(Error::Data(
                "unsupported LAS dimension: undocumented Extra Bytes record tail".into(),
            ));
        }
        Ok(result)
    }

    fn raw(&mut self, name: &str, kind: Scalar, at: usize) {
        self.dimensions.push(Dimension {
            name: name.into(),
            kind,
            decode: Decode::Raw(RAW + at),
        });
    }

    fn bits(&mut self, name: &str, at: usize, shift: u8, mask: u8) {
        self.dimensions.push(Dimension {
            name: name.into(),
            kind: Scalar::U8,
            decode: Decode::Bits {
                at: RAW + at,
                shift,
                mask,
            },
        });
    }

    pub fn validate(&self, raw: &[u8]) -> Result<(), Error> {
        // The four scratch-only fields have already been validated by ingestion.
        for dim in &self.dimensions[4..] {
            dim.validate(raw)?;
        }
        Ok(())
    }
}

pub(super) fn f64_at(bytes: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

pub(super) fn position(row: &[u8]) -> [f64; 3] {
    [f64_at(row, 0), f64_at(row, 8), f64_at(row, 16)]
}

fn text(bytes: &[u8]) -> Result<String, Error> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| Error::Data("LAS metadata string is not valid UTF-8".into()))
}

pub(super) fn header_crs(header: &las::Header) -> Result<String, Error> {
    let wkts: Vec<_> = header
        .all_vlrs()
        .filter(|v| v.user_id == "LASF_Projection" && v.record_id == 2112)
        .collect();
    if wkts.len() > 1 {
        return Err(Error::Data(
            "multiple LAS WKT CRS declarations are ambiguous".into(),
        ));
    }
    if let Some(vlr) = wkts.first() {
        let definition = text(&vlr.data)?;
        if definition.is_empty() {
            return Err(Error::Data("empty LAS WKT CRS declaration".into()));
        }
        return Ok(definition);
    }
    let keys: Vec<_> = header
        .all_vlrs()
        .filter(|v| v.user_id == "LASF_Projection" && v.record_id == 34735)
        .collect();
    if keys.len() > 1 {
        return Err(Error::Data(
            "multiple LAS GeoTIFF CRS declarations are ambiguous".into(),
        ));
    }
    if let Some(vlr) = keys.first() {
        let data = &vlr.data;
        let word = |at| u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
        if data.len() < 8
            || !data.len().is_multiple_of(2)
            || word(0) != 1
            || word(2) != 1
            || word(4) > 1
            || data.len() < 8 + word(6) as usize * 8
        {
            return Err(Error::Data("malformed LAS GeoTIFF key directory".into()));
        }
        let mut geographic = None;
        let mut projected = None;
        for at in (8..8 + word(6) as usize * 8).step_by(8) {
            let key = word(at);
            if !matches!(key, 1024 | 2048 | 3072 | 4096) {
                continue;
            }
            if word(at + 2) != 0 || word(at + 4) != 1 {
                return Err(Error::Data("malformed LAS CRS GeoKey value".into()));
            }
            let value = word(at + 6);
            if value == 0 {
                continue;
            }
            if (key == 1024 && value == 3) || key == 4096 {
                return Err(Error::Data("use a 2D horizontal CRS and explicit ellipsoidal height offset; compound/geocentric CRS is unsupported".into()));
            }
            if key == 3072 {
                projected = Some(value);
            }
            if key == 2048 {
                geographic = Some(value);
            }
        }
        if let Some(code) = projected.or(geographic) {
            if (1024..=32766).contains(&code) {
                return Ok(format!("EPSG:{code}"));
            }
            return Err(Error::Data("LAS has a user-defined/private GeoTIFF CRS; supply an explicit --sourceCrs override".into()));
        }
    }
    Err(Error::Data(
        "LAS has no CRS; supply --sourceCrs or explicitly choose local".into(),
    ))
}

pub(super) fn las_error(error: las::Error) -> Error {
    Error::Data(format!("invalid LAS/LAZ: {error}"))
}
