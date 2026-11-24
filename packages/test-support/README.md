# velune-test-support

Shared fixtures for the default, offline test layer described in [验证方式](../../docs/development/validation.md#测试分层). Crates use it only as a dev-dependency.

- `TempRoot`: a unique, canonical temporary directory removed on drop, including after a panic. Set `VELUNE_TEST_KEEP=1` to keep it and print its path for debugging.
- Files and fake executables: `write`, `executable`, `read`, `touch` relative to the root. A fake Harness is an executable script in the root; its protocol stays in the test that owns it.
- Serialization: `TempRoot` serializes as its path, so fixtures can embed it in JSON configs.
- Waiting: `wait_until` and `TempRoot::wait_for` poll a condition or a marker file with a deadline. Tests synchronize with fake processes through marker files instead of fixed sleeps.

The crate does not depend on any workspace crate. Fixtures that need a crate's own types, such as an opened application service, live in that crate's test modules and build on these primitives.
