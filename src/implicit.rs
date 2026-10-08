//! Availability and binary subtree writing for 3D Tiles 1.1 implicit tiling.
//!
//! Coordinates are relative to a subtree root, rather than the whole tileset.
//! This writer does not infer regular cells from an explicit hierarchy or change
//! content placement. Converters must establish those contracts separately.
//!
//! Implements the [implicit tiling specification](https://github.com/CesiumGS/3d-tiles/blob/main/specification/ImplicitTiling/README.adoc).
//! Like tyler's subtree writer, availability uses LSB-first bit vectors and
//! breadth-first levels with Morton order within each level. This implementation
//! is written independently; no tyler source is copied.

use crate::Error;
use bitvec::{order::Lsb0, vec::BitVec};
use morton_encoding::morton_encode;
use serde_json::{json, Value};
use std::collections::BTreeMap;

mod tileset;
pub use tileset::expand_tileset;
pub(crate) use tileset::write_tileset;

type Bits = BitVec<u8, Lsb0>;

/// Maximum combined availability storage per writer. Use smaller subtrees to
/// keep exponential level growth within this 16 MiB limit.
const MAX_AVAILABILITY_BYTES: usize = 16 * 1024 * 1024;

/// The regular subdivision of an implicit tileset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubdivisionScheme {
    Quadtree,
    Octree,
}

impl SubdivisionScheme {
    fn branches(self) -> usize {
        match self {
            Self::Quadtree => 4,
            Self::Octree => 8,
        }
    }

    /// Value for `implicitTiling.subdivisionScheme` in `tileset.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quadtree => "QUADTREE",
            Self::Octree => "OCTREE",
        }
    }
}

/// One tile's coordinates relative to its subtree root. The root is
/// `(level: 0, x: 0, y: 0, z: 0)`. Quadtrees require `z == 0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Coordinates {
    pub level: u32,
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// Builds one subtree's availability, including multiple content slots.
///
/// Tiles may be added in any order. Serialization rejects missing roots or
/// parents, including parents of child subtrees. Empty tiles are supported.
/// Content slots must match the implicit root's content URI templates.
pub struct Subtree {
    scheme: SubdivisionScheme,
    levels: u32,
    tiles: Bits,
    contents: Vec<Bits>,
    children: Bits,
    metadata: BTreeMap<usize, TileMetadata>,
}

/// Bounds and refinement error in the implicit root's local coordinate frame.
#[derive(Clone, Debug)]
pub struct TileMetadata {
    pub bounding_box: [f64; 12],
    pub geometric_error: f64,
    pub extras: Value,
}

fn invalid(message: &str) -> Error {
    Error::Data(format!("implicit subtree: {message}"))
}

impl Subtree {
    /// Allocate availability for `levels` tile levels and the immediately
    /// following child-subtree level. Zero levels and allocations above 16 MiB
    /// are refused. A zero content count creates a subtree with no content URI
    /// templates; its JSON omits `contentAvailability`.
    pub fn new(
        scheme: SubdivisionScheme,
        levels: u32,
        content_count: usize,
    ) -> Result<Self, Error> {
        if levels == 0 {
            return Err(invalid("subtreeLevels must be at least 1"));
        }
        let children = scheme
            .branches()
            .checked_pow(levels)
            .ok_or_else(|| invalid("subtreeLevels exceeds availability storage limit"))?;
        let tiles = (children - 1) / (scheme.branches() - 1);
        content_count
            .checked_add(1)
            .and_then(|count| tiles.div_ceil(8).checked_mul(count))
            .and_then(|bytes| bytes.checked_add(children.div_ceil(8)))
            .and_then(|bytes| {
                content_count
                    .checked_mul(std::mem::size_of::<Bits>())
                    .and_then(|overhead| bytes.checked_add(overhead))
            })
            .filter(|&bytes| bytes <= MAX_AVAILABILITY_BYTES)
            .ok_or_else(|| invalid("availability exceeds 16 MiB; use smaller subtrees"))?;
        Ok(Self {
            scheme,
            levels,
            tiles: Bits::repeat(false, tiles),
            contents: (0..content_count)
                .map(|_| Bits::repeat(false, tiles))
                .collect(),
            children: Bits::repeat(false, children),
            metadata: BTreeMap::new(),
        })
    }

    fn morton(&self, coordinates: Coordinates) -> Result<usize, Error> {
        let Coordinates { level, x, y, z } = coordinates;
        if level > self.levels {
            return Err(invalid("coordinates exceed the child-subtree level"));
        }
        let width = 1u32
            .checked_shl(level)
            .ok_or_else(|| invalid("coordinate level exceeds integer range"))?;
        if x >= width || y >= width || z >= width {
            return Err(invalid("coordinates are outside their level"));
        }
        // morton-encoding puts its LAST coordinate in the least significant
        // lane. 3D Tiles requires x, then y, then z, from least significant up.
        let index = match self.scheme {
            SubdivisionScheme::Quadtree => {
                if z != 0 {
                    return Err(invalid("quadtree coordinates require z = 0"));
                }
                u128::from(morton_encode([y, x]))
            }
            SubdivisionScheme::Octree => morton_encode([z, y, x]),
        };
        usize::try_from(index).map_err(|_| invalid("Morton index exceeds platform limits"))
    }

    /// Mark a tile available and set its content slots. Re-adding a tile replaces
    /// its content availability. No ancestors are inserted automatically.
    pub fn set_tile(&mut self, coordinates: Coordinates, contents: &[bool]) -> Result<(), Error> {
        if coordinates.level >= self.levels {
            return Err(invalid(
                "tile must be inside the subtree, before its child-subtree level",
            ));
        }
        if contents.len() != self.contents.len() {
            return Err(invalid("content slot count differs from subtree templates"));
        }
        let morton = self.morton(coordinates)?;
        let offset =
            (self.scheme.branches().pow(coordinates.level) - 1) / (self.scheme.branches() - 1);
        let index = offset + morton;
        self.tiles.set(index, true);
        for (bits, &available) in self.contents.iter_mut().zip(contents) {
            bits.set(index, available);
        }
        Ok(())
    }

    /// Mark a reachable subtree root available. Its local level must equal
    /// `subtreeLevels`; child availability contains only that boundary level.
    pub fn set_child_subtree(&mut self, coordinates: Coordinates) -> Result<(), Error> {
        if coordinates.level != self.levels {
            return Err(invalid("child subtree must be at the child-subtree level"));
        }
        let index = self.morton(coordinates)?;
        self.children.set(index, true);
        Ok(())
    }

    /// Assign standard TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR semantics.
    /// When metadata is used, every available tile must have a row.
    pub fn set_metadata(
        &mut self,
        coordinates: Coordinates,
        metadata: TileMetadata,
    ) -> Result<(), Error> {
        if coordinates.level >= self.levels
            || metadata.bounding_box.iter().any(|v| !v.is_finite())
            || !metadata.geometric_error.is_finite()
            || metadata.geometric_error < 0.
        {
            return Err(invalid("invalid tile metadata"));
        }
        let index = (self.scheme.branches().pow(coordinates.level) - 1)
            / (self.scheme.branches() - 1)
            + self.morton(coordinates)?;
        if !self.tiles[index] {
            return Err(invalid("metadata belongs to an unavailable tile"));
        }
        self.metadata.insert(index, metadata);
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        if !self.tiles[0] {
            return Err(invalid("root tile is unavailable"));
        }
        let branches = self.scheme.branches();
        let mut parent_offset = 0;
        let mut offset = 1;
        let mut width = branches;
        for _ in 1..self.levels {
            for morton in self.tiles[offset..offset + width].iter_ones() {
                if !self.tiles[parent_offset + morton / branches] {
                    return Err(invalid("available tile has an unavailable parent"));
                }
            }
            parent_offset = offset;
            offset += width;
            width *= branches;
        }
        for morton in self.children.iter_ones() {
            if !self.tiles[parent_offset + morton / branches] {
                return Err(invalid(
                    "available child subtree has an unavailable parent tile",
                ));
            }
        }
        Ok(())
    }

    /// Serialize a binary `.subtree`, with a little-endian 24-byte header,
    /// 8-byte-aligned buffer views and chunks, and zero unused availability bits.
    /// Constant streams omit their buffer views. Bytes are deterministic and
    /// independent of the order tiles were added.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let mut binary = Vec::new();
        let mut views = Vec::new();
        let mut doc = json!({
            "tileAvailability": availability(&self.tiles, &mut binary, &mut views),
            "childSubtreeAvailability": availability(&self.children, &mut binary, &mut views),
        });
        if !self.contents.is_empty() {
            doc["contentAvailability"] = Value::Array(
                self.contents
                    .iter()
                    .map(|bits| availability(bits, &mut binary, &mut views))
                    .collect(),
            );
        }
        if !self.metadata.is_empty() {
            if self.metadata.len() != self.tiles.count_ones() {
                return Err(invalid("metadata must cover every available tile"));
            }
            let boxes: Vec<u8> = self
                .metadata
                .values()
                .flat_map(|m| m.bounding_box.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            let errors: Vec<u8> = self
                .metadata
                .values()
                .flat_map(|m| m.geometric_error.to_le_bytes())
                .collect();
            let mut strings = Vec::new();
            let mut offsets = vec![0u8; 4];
            for metadata in self.metadata.values() {
                strings.extend_from_slice(serde_json::to_string(&metadata.extras)?.as_bytes());
                offsets.extend_from_slice(
                    &u32::try_from(strings.len())
                        .map_err(|_| invalid("metadata strings exceed 4 GiB"))?
                        .to_le_bytes(),
                );
            }
            doc["tileMetadata"] = json!(0);
            let table: crate::metadata::PropertyTable = serde_json::from_value(
                json!({"class":"rustyTile","count":self.metadata.len(),"properties":{
                    "boundingBox":{"values":buffer_view(&boxes,&mut binary,&mut views)},
                    "geometricError":{"values":buffer_view(&errors,&mut binary,&mut views)},
                    "extras":{"values":buffer_view(&strings,&mut binary,&mut views),"stringOffsets":buffer_view(&offsets,&mut binary,&mut views)}
                }}),
            )?;
            doc["propertyTables"] = serde_json::to_value([table])?;
        }
        if !binary.is_empty() {
            doc["buffers"] = json!([{"byteLength": binary.len()}]);
            doc["bufferViews"] = Value::Array(views);
        }
        let mut json = serde_json::to_vec(&doc)?;
        json.resize(json.len().next_multiple_of(8), b' ');
        binary.resize(binary.len().next_multiple_of(8), 0);
        let mut output = Vec::with_capacity(24 + json.len() + binary.len());
        output.extend_from_slice(b"subt");
        output.extend_from_slice(&1u32.to_le_bytes());
        output.extend_from_slice(&(json.len() as u64).to_le_bytes());
        output.extend_from_slice(&(binary.len() as u64).to_le_bytes());
        output.extend_from_slice(&json);
        output.extend_from_slice(&binary);
        Ok(output)
    }
}

fn buffer_view(bytes: &[u8], binary: &mut Vec<u8>, views: &mut Vec<Value>) -> usize {
    binary.resize(binary.len().next_multiple_of(8), 0);
    let offset = binary.len();
    binary.extend_from_slice(bytes);
    let index = views.len();
    views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()}));
    index
}

fn availability(bits: &Bits, binary: &mut Vec<u8>, views: &mut Vec<Value>) -> Value {
    let count = bits.count_ones();
    if count == 0 || count == bits.len() {
        return json!({"constant": usize::from(count != 0)});
    }
    binary.resize(binary.len().next_multiple_of(8), 0);
    let offset = binary.len();
    binary.extend_from_slice(bits.as_raw_slice());
    let index = views.len();
    views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": bits.len().div_ceil(8)}));
    json!({"bitstream": index, "availableCount": count})
}
