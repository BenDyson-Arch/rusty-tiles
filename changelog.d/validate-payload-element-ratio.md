### Changed

- `validate`: the default `totalPayloadElements` budget is now max(16 million, 25 × stored bytes), up from 16 ×. Indexed HLOD coarse tiles take fewer stored bytes for the same validation work, and the largest measured archive needs 16.1 elements per stored byte. The ratio is that maximum with a 1.5× margin.
