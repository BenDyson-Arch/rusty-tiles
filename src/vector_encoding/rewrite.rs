use super::{add, align, exact, limit, mul, CodecError, Encoded};
use super::{
    framing,
    plan::{Action, Known, Plan, View},
    raw::Span,
};
use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};
use std::mem::size_of;
#[derive(Clone, Copy)]
pub(super) enum Edit {
    Raw {
        offset: usize,
    },
    Encoded {
        offset: usize,
        length: usize,
        fallback_offset: usize,
    },
}
struct Buffer<'p, 'a> {
    plan: &'p Plan<'a>,
    length: usize,
}
impl Serialize for Buffer<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        for f in self.plan.arena.fields(self.plan.buffers[0]) {
            if f.key == "byteLength" {
                m.serialize_entry(f.key.as_ref(), &self.length)?;
            } else {
                m.serialize_entry(f.key.as_ref(), f.value)?;
            }
        }
        m.end()
    }
}
struct FallbackFlag;
impl Serialize for FallbackFlag {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry("fallback", &true)?;
        m.end()
    }
}
struct FallbackExtension;
impl Serialize for FallbackExtension {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry("EXT_meshopt_compression", &FallbackFlag)?;
        m.end()
    }
}
struct Fallback(usize);
impl Serialize for Fallback {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(2))?;
        m.serialize_entry("byteLength", &self.0)?;
        m.serialize_entry("extensions", &FallbackExtension)?;
        m.end()
    }
}
struct Buffers<'p, 'a> {
    p: &'p Plan<'a>,
    length: usize,
}
impl Serialize for Buffers<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut q = s.serialize_seq(Some(2))?;
        q.serialize_element(&Buffer {
            plan: self.p,
            length: self.length,
        })?;
        q.serialize_element(&Fallback(self.p.fallback_bytes))?;
        q.end()
    }
}
struct Meshopt {
    edit: Edit,
    action: Action,
}
impl Serialize for Meshopt {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let Edit::Encoded { offset, length, .. } = self.edit else {
            return Err(serde::ser::Error::custom("encoded edit required"));
        };
        let Action::Encode { stride, count, .. } = self.action else {
            return Err(serde::ser::Error::custom("encoded action required"));
        };
        let mut m = s.serialize_map(Some(7))?;
        m.serialize_entry("buffer", &0u8)?;
        m.serialize_entry("byteOffset", &offset)?;
        m.serialize_entry("byteLength", &length)?;
        m.serialize_entry("byteStride", &stride)?;
        m.serialize_entry("count", &count)?;
        m.serialize_entry("mode", "ATTRIBUTES")?;
        m.serialize_entry("filter", "NONE")?;
        m.end()
    }
}
struct Extensions<'p, 'a> {
    p: &'p Plan<'a>,
    span: Option<Span>,
    edit: Edit,
    action: Action,
}
impl Serialize for Extensions<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        if let Some(span) = self.span {
            for f in self.p.arena.fields(span) {
                m.serialize_entry(f.key.as_ref(), f.value)?;
            }
        }
        m.serialize_entry(
            "EXT_meshopt_compression",
            &Meshopt {
                edit: self.edit,
                action: self.action,
            },
        )?;
        m.end()
    }
}
struct ViewJson<'p, 'a> {
    p: &'p Plan<'a>,
    view: &'p View,
    edit: Edit,
}
impl Serialize for ViewJson<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let (encoded, offset) = match self.edit {
            Edit::Raw { offset } => (false, offset),
            Edit::Encoded {
                fallback_offset, ..
            } => (true, fallback_offset),
        };
        let ext = Extensions {
            p: self.p,
            span: self.view.extensions,
            edit: self.edit,
            action: self.view.action,
        };
        let mut m = s.serialize_map(None)?;
        let mut had_offset = false;
        let mut had_ext = false;
        for f in self.p.arena.fields(self.view.fields) {
            match f.key.as_ref() {
                "buffer" => m.serialize_entry(f.key.as_ref(), &u8::from(encoded))?,
                "byteOffset" => {
                    had_offset = true;
                    m.serialize_entry(f.key.as_ref(), &offset)?;
                }
                "extensions" if encoded => {
                    had_ext = true;
                    m.serialize_entry(f.key.as_ref(), &ext)?;
                }
                _ => m.serialize_entry(f.key.as_ref(), f.value)?,
            }
        }
        if !had_offset {
            m.serialize_entry("byteOffset", &offset)?;
        }
        if encoded && !had_ext {
            m.serialize_entry("extensions", &ext)?;
        }
        m.end()
    }
}
struct Views<'p, 'a> {
    p: &'p Plan<'a>,
    edits: &'p [Edit],
}
impl Serialize for Views<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut q = s.serialize_seq(Some(self.edits.len()))?;
        for (view, edit) in self.p.views.iter().zip(self.edits) {
            q.serialize_element(&ViewJson {
                p: self.p,
                view,
                edit: *edit,
            })?;
        }
        q.end()
    }
}
struct Names<'p, 'a> {
    names: &'p [Option<super::plan::Declaration<'a>>],
}
impl Serialize for Names<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut q = s.serialize_seq(None)?;
        let mut found = false;
        for d in self.names {
            let d = d.unwrap();
            found |= d.kind == Known::Meshopt;
            q.serialize_element(d.raw)?;
        }
        if !found {
            q.serialize_element("EXT_meshopt_compression")?;
        }
        q.end()
    }
}
struct Root<'p, 'a> {
    p: &'p Plan<'a>,
    edits: &'p [Edit],
    bin_length: usize,
}
impl Serialize for Root<'_, '_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let d = &self.p.declarations;
        let used = Names {
            names: &d.used[..d.used_len],
        };
        let required = Names {
            names: &d.required[..d.required_len],
        };
        let mut m = s.serialize_map(None)?;
        let mut had_used = false;
        let mut had_required = false;
        for f in self.p.arena.fields(self.p.root) {
            match f.key.as_ref() {
                "buffers" => m.serialize_entry(
                    f.key.as_ref(),
                    &Buffers {
                        p: self.p,
                        length: self.bin_length,
                    },
                )?,
                "bufferViews" => m.serialize_entry(
                    f.key.as_ref(),
                    &Views {
                        p: self.p,
                        edits: self.edits,
                    },
                )?,
                "extensionsUsed" => {
                    had_used = true;
                    m.serialize_entry(f.key.as_ref(), &used)?;
                }
                "extensionsRequired" => {
                    had_required = true;
                    m.serialize_entry(f.key.as_ref(), &required)?;
                }
                _ => m.serialize_entry(f.key.as_ref(), f.value)?,
            }
        }
        if !had_used {
            m.serialize_entry("extensionsUsed", &used)?;
        }
        if !had_required {
            m.serialize_entry("extensionsRequired", &required)?;
        }
        m.end()
    }
}
#[cfg(test)]
thread_local! {static CAPACITY_ONE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static NATIVE_REACHED:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
pub(crate) fn with_capacity_one<T>(f: impl FnOnce() -> T) -> (T, usize) {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            CAPACITY_ONE.with(|c| c.set(self.0));
        }
    }
    let previous = CAPACITY_ONE.with(|c| c.replace(true));
    let _reset = Reset(previous);
    NATIVE_REACHED.with(|c| c.set(0));
    let result = f();
    (result, NATIVE_REACHED.with(|c| c.get()))
}
pub(crate) fn encode<E>(
    plan: Plan<'_>,
    checkpoint: &mut impl FnMut() -> std::result::Result<(), E>,
) -> std::result::Result<Encoded, CodecError<E>> {
    checkpoint().map_err(CodecError::Checkpoint)?;
    if plan.identity {
        let source = plan.source;
        let mut receipt = plan.receipt;
        let mut working = plan.working;
        let maximum = plan.limits.values.candidate_bytes;
        if source.len() > maximum {
            return Err(limit("identity candidate byte limit").into());
        }
        drop(plan);
        #[cfg(test)]
        super::tests::identity_copy();
        working.admit(source.len())?;
        let mut bytes = exact(source.len())?;
        bytes.extend_from_slice(source);
        receipt.after_bytes = bytes.len();
        receipt.estimated_peak_bytes = working.peak;
        checkpoint().map_err(CodecError::Checkpoint)?;
        return Ok(Encoded { bytes, receipt });
    }
    let mut edits = exact(plan.views.len())?;
    let mut bin: Vec<u8> = exact(plan.bin_bound)?;
    bin.resize(plan.bin_bound, 0);
    let mut cursor = 0usize;
    let mut fallback = 0usize;
    for view in &plan.views {
        checkpoint().map_err(CodecError::Checkpoint)?;
        cursor = align(cursor, 8)?;
        fallback = align(fallback, 8)?;
        match view.action {
            Action::Raw => {
                let end = add(cursor, view.source.len)?;
                bin[cursor..end].copy_from_slice(
                    &plan.bin[view.source.offset..view.source.offset + view.source.len],
                );
                edits.push(Edit::Raw { offset: cursor });
                cursor = end;
            }
            Action::Encode {
                stride,
                count,
                bound,
            } => {
                let destination_size = bound;
                #[cfg(test)]
                let destination_size = {
                    let size = CAPACITY_ONE.with(|c| if c.get() { 1 } else { destination_size });
                    NATIVE_REACHED.with(|c| c.set(c.get() + 1));
                    size
                };
                let encoded = unsafe {
                    meshopt::ffi::meshopt_encodeVertexBuffer(
                        bin[cursor..cursor + bound].as_mut_ptr(),
                        destination_size,
                        plan.bin[view.source.offset..view.source.offset + view.source.len]
                            .as_ptr()
                            .cast(),
                        count,
                        stride,
                    )
                };
                if encoded == 0 || encoded > bound {
                    return Err(CodecError::EncodingFailure("ATTRIBUTES encoder"));
                }
                edits.push(Edit::Encoded {
                    offset: cursor,
                    length: encoded,
                    fallback_offset: fallback,
                });
                cursor = add(cursor, encoded)?;
            }
            Action::Existing { .. } => unreachable!("existing streams select identity"),
        };
        fallback = add(fallback, view.source.len)?;
    }
    bin.truncate(cursor);
    let (bytes, working) = {
        let root = Root {
            p: &plan,
            edits: &edits,
            bin_length: if plan.metadata {
                align(bin.len(), 8)?
            } else {
                bin.len()
            },
        };
        let mut count = framing::Count::new(plan.json_bound.min(plan.limits.values.json_bytes));
        if serde_json::to_writer(&mut count, &root).is_err() {
            return Err(if count.exceeded {
                limit("rewritten JSON byte/bound limit").into()
            } else {
                CodecError::EncodingFailure("JSON count")
            });
        }
        let (jpad, _, total) = framing::size(count.n, bin.len(), plan.metadata)?;
        if jpad > plan.limits.values.json_bytes {
            return Err(limit("rewritten padded JSON byte limit").into());
        }
        if total > plan.limits.values.candidate_bytes {
            return Err(limit("candidate GLB byte limit").into());
        }
        let mut working = plan.working;
        let retained = add(
            add(
                mul(plan.arena.fields.capacity(), size_of::<super::raw::Field>())?,
                mul(plan.views.capacity(), size_of::<View>())?,
            )?,
            plan.arena.owned_keys(),
        )?;
        working.admit(add(
            add(
                add(
                    add(retained, size_of::<Plan>())?,
                    mul(edits.capacity(), size_of::<Edit>())?,
                )?,
                bin.capacity(),
            )?,
            total,
        )?)?;
        let bytes = framing::emit(&root, &bin, count.n, plan.metadata)?;
        (bytes, working)
    };
    let mut receipt = plan.receipt;
    receipt.after_bytes = bytes.len();
    receipt.estimated_peak_bytes = working.peak;
    drop(edits);
    drop(bin);
    drop(plan);
    checkpoint().map_err(CodecError::Checkpoint)?;
    Ok(Encoded { bytes, receipt })
}
