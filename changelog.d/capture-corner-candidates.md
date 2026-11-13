### Changed

- Mesh capture joins corner depths over the leaves directly. Only corners at
  positions another leaf's box and occupancy cells can hold are sorted, and
  the weld-table sort and the two full corner sorts are gone. Output is
  identical; a stop request is honoured before each leaf.
