//! The limit surface (VS-C): the configurable keys, how each effective limit is derived from
//! the admitted stored bytes S and the request's overrides, and structured limit refusals.
use super::types::ValidationLimits;
use serde::Serialize;
use std::{collections::BTreeMap, fmt, str::FromStr};

/// The eleven configurable limits (VS-C-3): four opt-in volume caps (Class B) and seven
/// amplification budgets (Class C). Per-item format limits (Class A) are not configurable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LimitKey {
    SourceArchiveBytes,
    ArchiveEntries,
    CentralDirectoryBytes,
    ArchiveStoredBytes,
    HierarchyVisits,
    References,
    TotalBytesRead,
    TotalPayloadElements,
    ContainmentWorkUnits,
    ContainmentBytesRead,
    ContainmentDecodedBytes,
}

impl LimitKey {
    pub const ALL: [Self; 11] = [
        Self::SourceArchiveBytes,
        Self::ArchiveEntries,
        Self::CentralDirectoryBytes,
        Self::ArchiveStoredBytes,
        Self::HierarchyVisits,
        Self::References,
        Self::TotalBytesRead,
        Self::TotalPayloadElements,
        Self::ContainmentWorkUnits,
        Self::ContainmentBytesRead,
        Self::ContainmentDecodedBytes,
    ];

    /// The camelCase key of the report's `limits`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceArchiveBytes => "sourceArchiveBytes",
            Self::ArchiveEntries => "archiveEntries",
            Self::CentralDirectoryBytes => "centralDirectoryBytes",
            Self::ArchiveStoredBytes => "archiveStoredBytes",
            Self::HierarchyVisits => "hierarchyVisits",
            Self::References => "references",
            Self::TotalBytesRead => "totalBytesRead",
            Self::TotalPayloadElements => "totalPayloadElements",
            Self::ContainmentWorkUnits => "containmentWorkUnits",
            Self::ContainmentBytesRead => "containmentBytesRead",
            Self::ContainmentDecodedBytes => "containmentDecodedBytes",
        }
    }

    /// Class B: an absolute volume cap, unset by default.
    pub fn is_cap(self) -> bool {
        (self as usize) < 4
    }
}

impl fmt::Display for LimitKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A key that is not one of the eleven configurable limits (unknown, or a fixed per-item
/// format limit).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0} is not a configurable limit (allowed: sourceArchiveBytes, archiveEntries, centralDirectoryBytes, archiveStoredBytes, hierarchyVisits, references, totalBytesRead, totalPayloadElements, containmentWorkUnits, containmentBytesRead, containmentDecodedBytes)")]
pub struct UnknownLimitKey(pub String);

impl FromStr for LimitKey {
    type Err = UnknownLimitKey;
    fn from_str(key: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == key)
            .ok_or_else(|| UnknownLimitKey(key.to_owned()))
    }
}

/// How an effective limit was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Derivation {
    /// A Class C budget at its floor (`⌊S·p/q⌋ ≤ floor`).
    Floor,
    /// A Class C budget at `⌊S·p/q⌋ > floor`.
    Scaled,
    /// A Class C budget given by the request.
    Explicit,
    /// A Class B volume cap given by the request.
    Cap,
}

impl Derivation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Floor => "floor",
            Self::Scaled => "scaled",
            Self::Explicit => "explicit",
            Self::Cap => "cap",
        }
    }
}

/// The rule of one Class C budget in the report: `max(floor, ⌊S·p/q⌋)`, or `floor + ⌊S·p/q⌋`
/// when `additive`, unless explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetRule {
    pub floor: u64,
    pub per_stored_byte: [u64; 2],
    pub source: Derivation,
    /// The floor is added to the per-byte term instead of bounding it from below.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub additive: bool,
}

/// The report's `limitDerivation`: the model version and each Class C budget's rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LimitDerivation {
    pub model: u32,
    pub budgets: BTreeMap<&'static str, BudgetRule>,
}

/// The rule table (VS-C-1). Bump on any change to a floor or ratio (VS-D freezes κ).
pub const LIMIT_MODEL: u32 = 1;
/// Class C: key, floor, κ = p/q per stored byte, and whether the floor is added.
/// `totalPayloadElements` is `16,000,000 + 130·S`: the form of its time statement, so legitimate
/// producer output of any size validates with a 1.5× margin (validator-limits-amendment.md).
const BUDGETS: [(LimitKey, u64, u64, u64, bool); 7] = [
    (LimitKey::HierarchyVisits, 1 << 18, 1, 8, false),
    (LimitKey::References, 1 << 20, 1, 8, false),
    (LimitKey::TotalBytesRead, 1 << 31, 4, 1, false),
    (LimitKey::TotalPayloadElements, 16_000_000, 130, 1, true),
    (LimitKey::ContainmentWorkUnits, 1 << 29, 128, 1, false),
    (LimitKey::ContainmentBytesRead, 1 << 31, 4, 1, false),
    (LimitKey::ContainmentDecodedBytes, 1 << 31, 16, 1, false),
];

/// `⌊S·p/q⌋` in u128, saturating at `u64::MAX`.
fn per_stored_byte(stored_bytes: u64, p: u64, q: u64) -> u64 {
    u64::try_from(u128::from(stored_bytes) * u128::from(p) / u128::from(q)).unwrap_or(u64::MAX)
}

impl ValidationLimits {
    fn slot(&mut self, key: LimitKey) -> LimitSlot<'_> {
        match key {
            LimitKey::SourceArchiveBytes => LimitSlot::Cap(&mut self.source_archive_bytes),
            LimitKey::ArchiveEntries => LimitSlot::Cap(&mut self.archive_entries),
            LimitKey::CentralDirectoryBytes => LimitSlot::Cap(&mut self.central_directory_bytes),
            LimitKey::ArchiveStoredBytes => LimitSlot::Cap(&mut self.archive_stored_bytes),
            LimitKey::HierarchyVisits => LimitSlot::Budget(&mut self.hierarchy_visits),
            LimitKey::References => LimitSlot::Budget(&mut self.references),
            LimitKey::TotalBytesRead => LimitSlot::Budget(&mut self.total_bytes_read),
            LimitKey::TotalPayloadElements => LimitSlot::Budget(&mut self.total_payload_elements),
            LimitKey::ContainmentWorkUnits => LimitSlot::Budget(&mut self.containment_work_units),
            LimitKey::ContainmentBytesRead => LimitSlot::Budget(&mut self.containment_bytes_read),
            LimitKey::ContainmentDecodedBytes => {
                LimitSlot::Budget(&mut self.containment_decoded_bytes)
            }
        }
    }

    /// The effective value of a configurable key (`None`: an unset cap).
    pub fn get(&self, key: LimitKey) -> Option<u64> {
        match key {
            LimitKey::SourceArchiveBytes => self.source_archive_bytes,
            LimitKey::ArchiveEntries => self.archive_entries,
            LimitKey::CentralDirectoryBytes => self.central_directory_bytes,
            LimitKey::ArchiveStoredBytes => self.archive_stored_bytes,
            LimitKey::HierarchyVisits => Some(self.hierarchy_visits),
            LimitKey::References => Some(self.references),
            LimitKey::TotalBytesRead => Some(self.total_bytes_read),
            LimitKey::TotalPayloadElements => Some(self.total_payload_elements),
            LimitKey::ContainmentWorkUnits => Some(self.containment_work_units),
            LimitKey::ContainmentBytesRead => Some(self.containment_bytes_read),
            LimitKey::ContainmentDecodedBytes => Some(self.containment_decoded_bytes),
        }
    }
}

enum LimitSlot<'a> {
    Cap(&'a mut Option<u64>),
    Budget(&'a mut u64),
}

/// How each configurable key of one validation was derived, indexed by `LimitKey as usize`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sources([Derivation; 11]);

impl Sources {
    /// Every key given absolutely (the in-crate controls' explicit `ValidationLimits`).
    pub(crate) fn absolute() -> Self {
        Self(LimitKey::ALL.map(|key| {
            if key.is_cap() {
                Derivation::Cap
            } else {
                Derivation::Explicit
            }
        }))
    }

    pub(crate) fn of(&self, key: LimitKey) -> Derivation {
        self.0[key as usize]
    }
}

/// The request's overrides, resolved once at the operation boundary (VS-C-3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LimitPlan {
    overrides: [Option<u64>; 11],
}

impl LimitPlan {
    pub(crate) fn set(&mut self, key: LimitKey, value: u64) {
        self.overrides[key as usize] = Some(value);
    }

    /// The limits known before the catalog exists: Class A and the caps. Budgets are not read
    /// before `resolve`.
    pub(crate) fn admission(&self) -> ValidationLimits {
        let mut limits = ValidationLimits::default();
        for key in LimitKey::ALL.into_iter().filter(|key| key.is_cap()) {
            if let LimitSlot::Cap(cap) = limits.slot(key) {
                *cap = self.overrides[key as usize];
            }
        }
        limits
    }

    /// The effective limits and their derivation for `stored_bytes` admitted stored bytes:
    /// a pure function of (S, overrides, model).
    pub(crate) fn resolve(
        &self,
        stored_bytes: u64,
    ) -> (ValidationLimits, LimitDerivation, Sources) {
        let mut limits = self.admission();
        let mut sources = Sources::absolute();
        let mut budgets = BTreeMap::new();
        for (key, floor, p, q, additive) in BUDGETS {
            let scaled = per_stored_byte(stored_bytes, p, q);
            let (value, source) = match self.overrides[key as usize] {
                Some(value) => (value, Derivation::Explicit),
                None if additive && scaled > 0 => {
                    (floor.saturating_add(scaled), Derivation::Scaled)
                }
                None if !additive && scaled > floor => (scaled, Derivation::Scaled),
                None => (floor, Derivation::Floor),
            };
            if let LimitSlot::Budget(slot) = limits.slot(key) {
                *slot = value;
            }
            sources.0[key as usize] = source;
            budgets.insert(
                key.as_str(),
                BudgetRule {
                    floor,
                    per_stored_byte: [p, q],
                    source,
                    additive,
                },
            );
        }
        let derivation = LimitDerivation {
            model: LIMIT_MODEL,
            budgets,
        };
        (limits, derivation, sources)
    }
}

/// The derivation reported for explicit test limits: every budget explicit.
#[cfg(test)]
pub(crate) fn absolute_derivation() -> LimitDerivation {
    LimitDerivation {
        model: LIMIT_MODEL,
        budgets: BUDGETS
            .into_iter()
            .map(|(key, floor, p, q, additive)| {
                (
                    key.as_str(),
                    BudgetRule {
                        floor,
                        per_stored_byte: [p, q],
                        source: Derivation::Explicit,
                        additive,
                    },
                )
            })
            .collect(),
    }
}

/// A refused charge of a configurable limit (VS-C-5 T3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitExceeded {
    /// The camelCase key.
    #[serde(serialize_with = "key_name")]
    pub limit: LimitKey,
    /// The effective value of the key.
    pub value: u64,
    /// The total the refused charge would have reached (saturating), or the capped quantity.
    pub observed: u64,
    pub derivation: Derivation,
}

fn key_name<S: serde::Serializer>(key: &LimitKey, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(key.as_str())
}

/// A resource-limit refusal: its message and, for a configurable limit, the structured cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitRefusal {
    message: String,
    pub limit: Option<LimitExceeded>,
}

impl LimitRefusal {
    /// The message without the structured suffix.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<String> for LimitRefusal {
    fn from(message: String) -> Self {
        Self {
            message,
            limit: None,
        }
    }
}

impl LimitRefusal {
    pub(crate) fn exceeded(message: impl Into<String>, limit: LimitExceeded) -> Self {
        Self {
            message: message.into(),
            limit: Some(limit),
        }
    }
}

impl fmt::Display for LimitRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(limit) = &self.limit {
            write!(
                f,
                " (limit {}={}, {})",
                limit.limit,
                limit.value,
                limit.derivation.as_str()
            )?;
        }
        Ok(())
    }
}
