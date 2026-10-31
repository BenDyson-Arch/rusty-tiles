### Changed

- HLOD bakes spend less CPU after the S1b border guards: the nearest-source searches behind S2, D4 representatives and the moved-boundary witnesses avoid redundant heap traffic, node copies and NaN fix-ups, and fold checks group faces and edges more cheaply. Visit order, search budgets and every guard are unchanged, and output is byte-identical.
