### Changed

- Add one source-admitted descendant geometry proposal at E0 = 2.8 m, retaining
  exact boundary support and measured child-monotone error. Refusal keeps the
  accepted tile; the ownership root is reserved for root competition.
- Admit coarse appearance up to 2048² per semantic slot with c = 1.25,
  measured source-spacing floors and explicit density/cap reporting.
- Admit page workspace at its final layout size, borrowing from the source cache
  with a bounded wait instead of reserving 2048² for every worker.

- Cache the query-independent sliver classification for source triangles without changing sampled witnesses or their memory charge.
