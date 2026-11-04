### Fixed

- A lossy texture page whose encode misses the codec's own verification floor no longer fails the run. This is the decoded-level error or detail correlation check, for example "detail does not follow the input" on sharp synthetic patterns. The page escalates through the same ladder as a gate miss. Every step still runs the full codec verification and the gates. Structural verification failures stay hard errors. The report counts escalated pages by trigger (`escalation_reasons`: `gate`, `codec_verify`), and each listed miss names its reason.
