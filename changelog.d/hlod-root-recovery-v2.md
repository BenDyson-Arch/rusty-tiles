### Changed

- HLOD: an omission no longer costs the root, whatever the size of the frontier below it
  (root recovery v2). Every omitted coarse cell passes up an unpublished stand-in, an N3
  reduction of its inputs to the cap with no border contract, and the root builds over its
  built children and those stand-ins. The root's G is measured against the source as for
  any tile. It is delivered over the whole published frontier (at most 384 tiles), with the
  omitted cells bypassed, and declares recovery version 2. Its declared drops are counted
  against that frontier, as the validator recounts them. Version 1, which rebuilt the root
  only from a complete frontier of at most 8 tiles, is no longer produced; the validator
  still accepts it. Where every cell builds, the archive is unchanged.
- Validation accepts recovery version 2: a frontier of at most 384 tiles with no total
  input bound, and a tileset root with more children than the subdivision scheme's
  branches when it declares that relation.
