### Changed

- CI: the Rust job runs the library unit tests in parallel. Running them on one thread took about 21 of the job's 37 minutes and pushed it past its 40-minute limit. The C1 context controls still run serially, because the payload-context receipt needs their stderr lines intact, as does the one library test that changes the working directory. Binaries, integration tests and doctests also stay serial.
