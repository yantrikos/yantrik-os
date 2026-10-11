# #588 review fixes

- SHOULD-FIX 1: `control_files_create::make_checked` runs in the worker. For a mind it canonicalizes `dir` (and the home) and re-runs `make_verdict` (home_paths `may_create` and the `here` check) on the resolved path, refusing if the verdict changed, then creates under the resolved path. Who is calling is read on the UI thread and carried into the closure. Tests: a folder swapped for a link out of the home is refused and nothing is made through it; a folder still in the home is made. Residual: a swap between canonicalize and create is not closed (no openat/mkdirat).
- SHOULD-FIX 2: `tests/smoke/check_results.json` holds only action names and `grade_declared`; nothing reads `requested_folder`, `requested_file` or `now`. No change needed.
- SHOULD-FIX 3: the duplicate `files_new_folder`/`files_new_file` row in docs/app-control.md is merged into the Files row.
- Grades: neither action sets `.risk(..)`, before or after; both stay at the default `standard`. `grade_tests` pins that deferring never regrades, the default is `standard`, and neither action declares a risk.
- NIT 1: an entry that vanished after EEXIST now answers `kind: unknown`.
- Tests: harness 111 passed; yantrik-ui bin 1165 passed, 1 failed (`harness_install::tests::a_coloured_installer_reaches_the_row_as_plain_text`, environmental: the sandbox shell prints "nvm"; file untouched by this PR).
- Not done: NIT 2 (the settled test still scans source).
