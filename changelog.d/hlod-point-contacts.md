### Fixed

- HLOD: a tile touching another tile's shared border chain at one vertex only now keeps that point in its border contract. Before, it could close a T-junction edge onto the neighbour's edge, and an ancestor then failed `--expect-profile hlod` validation (`interfaceWinding`).
