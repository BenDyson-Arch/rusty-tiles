### Changed

- Mesh capture's preflight scan reads each used position once, in file order,
  after marking the used vertices from the index buffer, and payload
  validation runs on the capture workers. An invalid source still reports
  the same first error. Output is unchanged.
