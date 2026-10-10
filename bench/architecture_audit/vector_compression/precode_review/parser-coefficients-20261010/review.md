# Narrow parser coefficient review

Disposition: **ACCEPT SOURCE-BACKED CORE COEFFICIENTS FOR MATCHED `json::admit`; FIXED SLACK AND INTEGRATED DEFAULTS REMAIN SCOPED ENGINEERING GATES.** This additive review does not approve the mutable owned-Plan prototype or production implementation. It does not replace earlier frozen reviews or the failed original `2J` Plan calibration.

Inputs are pinned in `input-pins.json`. The accepted C1 JSON helper is `dcd5c49212c5f4c7e1812de1205f8e098866d52d8dc5c58e37871c489ffe7e18`. The actual P2 artifact receipt remains the earlier 205-artifact execution binding; this review adds source-only pins for the installed Rust 1.98 library machinery. No Cargo, rustc, native executable, target or Git command was run. `table-model.py` is an independent integer model; its output is arithmetic evidence, not allocator evidence.

Let `J` be the actual admitted JSON source length and `K_upper=min(value_node_limit,J)`. The estimate under inspection is `8J + 256K_upper + 64KiB`, calculated with checked arithmetic before JSON admission. The owned Plan must use actual admitted `K`, separately. Calling the finite `Value` adapter is outside this ledger.

The target assumptions are ordinary Global requested allocation sizes, the matched native 64-bit target, `Slot` size 24, `Cow<str>` size 24/alignment 8, and the std hashbrown control group of at most 16 bytes. Existing actual P2 records report the two type sizes. The new prototype must assert them again against its actual compiler/closure. This is a requested-owner storage estimate with conservative old/new overlap; it excludes allocator bookkeeping and process RSS.

## Slot owners: at most 72K_upper

C1 `State::reserve` checks `required<=max_slots`, chooses `min(max(2capacity,required),max_slots)`, and calls `try_reserve_exact`. The pinned RawVec `grow_exact` requests `len+additional` without the amortized minimum; Global returns the requested size. Therefore each pending or immediate-child vector capacity is at most `K_upper`.

One container decoder and its immediate-child vector exist at a time. During child growth the live owners are pending, child-old and child-new. After that decoder/key set drops, pending growth can overlap pending-old, pending-new and children. Each case has at most three bounded backing allocations: `3*24*K_upper=72K_upper`. This deliberately avoids any tighter historical-node argument. `pending.extend(children.into_iter().rev())` fits the reservation already secured for the exact child length; the pinned Vec implementations reserve only the iterator length or grow when full. Slots contain borrowed RawValue references and depths, with no cloned source payload.

## Unique-key table: at most 116K_upper

The pinned std HashSet wraps HashMap, whose std dependency is vendored hashbrown 0.17.1. Its table stores `(Cow<str>,())`; it is not the separate Cargo hashbrown dependency. With 24-byte entries and Group16, table layout is `L(b)=25b+16` for power-of-two bucket count `b>=4`. Small capacities use 4/8 buckets; larger buckets keep 1/8 empty. C1 never deletes table entries, rejects a duplicate before insertion, and fallibly reserves one key at a time.

For an existing table of `b` buckets, growth requires `m=capacity(b)+1` keys and allocates `2b` buckets while the old table remains live. Old plus new storage is `75b+32`; capacity is `b-1` for b=4/8 and `7b/8` thereafter. Including the initial 116-byte four-bucket table at m=1, this is at most `116m`. Resize uses `ptr::copy_nonoverlapping` for entry headers and moves ownership without cloning escaped-key buffers. Retained table storage also fits this bound.

A new key is decoded before its Child node is charged. This does not invalidate `m<=K_upper`: prior completed map children plus the already charged parent account for the extra current key, including the node-limit failure path. A duplicate's temporary owned key is a separate source occurrence; its bytes are covered by the payload term below. The independent integer model verifies every m=1..65536 and records all growth thresholds. First peaks are m1=116, m4=332, m8=632, m15=1232. This finite enumeration supports the algebra, not universal actual allocator behavior.

## Scratch and escaped key payload: at most 4J+8

C1 holds only one serde decoder. Root RawValue grammar admission drops its decoder before pending work. Each container decoder/key set drops before descent; scalar strings use their own decoder only after container completion. RawValue children borrow source; scalar numeric lexemes are skipped without float/Value materialization.

Pinned serde has one u8 scratch Vec reused by string decoding and iterative raw nesting skip. `clear` preserves its capacity. Decoded strings do not expand relative to their JSON source occurrence; the nesting stack uses one byte per source opening container. Thus maximum required scratch length is at most J. RawVec amortized growth chooses `max(2old,required,8)` for u8. At a realloc, old is smaller than required; old+new is at most `3J+8`, including the minimum first allocation. Larger retained capacity from an earlier string/skip still fits that same source bound.

C1's copied escaped keys use `String::new` plus `try_reserve_exact(decoded_len)`, followed by a fitting `push_str`. Keys within the single current object occupy distinct source occurrences, and decoded lengths do not exceed those occurrences. All simultaneously retained escaped key payload, including the next key/duplicate temporary, is at most J. Combining that payload with scratch gives `4J+8`. The original late-escape actual calibration remains evidence that a naive Plan 2J term was insufficient; it does not contradict this parser ledger.

## What the remaining slack means

The core source-owned backing allocations above sum to at most `4J+188K_upper+8`. The proposed estimate has another `4J+68K_upper` plus nearly 64KiB of fixed headroom. C1's custom messages are finite static strings; SliceRead/StrRead grammar error codes have finite text and bounded usize position formatting, without echoing arbitrary source strings. RandomState uses fixed scalar keys and a scalar thread-local cell; its native randomness dispatch is recorded separately as a platform/runtime boundary. These source observations support a generous finite allowance, but this review does not claim an exact universal 64KiB bound for process initialization, OS/libc interposition, allocator abort machinery, or future error/consumer changes.

The meaningful next actual probes are successful and refused maps at key counts 1/3/4/7/8/14/15, high-K wide maps, alternating pending/child growth, deep raw skip before depth refusal, escaped duplicate keys and the late-escape key. The coordinator must verify measured parser phase peaks against the declared estimate and preserve failed records. Actual Plan, consumer/path/receipt/source capture, native buffer work, output framing, defaults and P2–P5 integration still need their separate ownership ledgers and acceptance.
