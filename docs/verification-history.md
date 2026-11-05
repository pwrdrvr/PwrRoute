# Verification history

## Retired Sandboxer workflow measurements (2026-10-29)

These are historical results from the removed Sandboxer verification skill.
The provider workflow and its `verify.sh` helper are retired; this record
does not establish results for the current commit.

The measured `large` Ubuntu 24.04 x64 sandbox had 8 vCPUs and 15 GB of RAM.

| Step | Historical command | Cold `target/` | From the snapshot, one crate edited |
| --- | --- | --- | --- |
| Setup | `verify.sh setup` | 58 s | 1 s |
| Typecheck job | `verify.sh lint` | 112 s | 47 s |
| Test job | `verify.sh test` | 176 s | 32 s |
| Linux Desktop E2E | `verify.sh e2e` | 95 s | 15 s |

"Cold" compiled every crate. The base snapshot held a `target/` built at
its commit, so a run there recompiled only what the change touched. The last
column edited `pwrroute-engine`, which most crates depended on. The tests
were 1,045 workspace tests and 15 E2E tests, all passing at both points.

The workflow disabled dev/test debug info, as CI did. Default-browser
registration scenarios were skipped because `PWRROUTE_E2E_DISPOSABLE` was
unset. These checks covered Linux only.
