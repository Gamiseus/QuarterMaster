# Testing QuarterMaster

`.\check.ps1` runs everything below in order and stops at the first failure. Nothing here starts the game.

Integration, launcher and legacy app tests are kept on our machines only: `.gitignore` leaves out `crates/*/tests/`, `tools/*/tests/`, `launcher-ui/tests/` and `ui/tests/`. Unit tests beside the code are in the repository, so a fresh clone still runs `cargo test --workspace`. Back the ignored folders up yourself; git does not hold them.

## Where the tests are

| What | Where | Run it |
|---|---|---|
| Rust unit tests | Beside the code: `crates/<crate>/src/**`, in `#[cfg(test)]` modules or `*_tests.rs` / `tests.rs` files next to the module they test | `cargo test -p <crate> --lib` |
| Rust integration tests | `crates/<crate>/tests/suite/`: one program per crate (`main.rs`) with one file per area. Shared helpers in `suite/common/` (qm-core) and `suite/support/` (qm-service). qm-manager has a single file, `tests/real_mods.rs` | `cargo test -p <crate> --test suite`; one area: `cargo test -p <crate> --test suite <area>::` |
| Loader runtime (Lua) | `crates/qm-core/src/compile.rs` runs QuarterMaster's own Lua on the game's `lua51.dll`; `crates/qm-core/testdata/faction_harness.lua` runs the faction runtime against functions copied from `decompile/`. Both skip when the DLL or the decompile is missing | `cargo test -p qm-core --lib compile::` |
| Site builder | `tools/qm-site-builder/tests/` | `cargo test -p qm-site-builder` |
| Launcher unit tests | `launcher-ui/tests/unit/*.test.ts` (Vitest). Fixtures are the `*-fixtures.ts` files and `helpers.ts` in the same folder | `npm test` in `launcher-ui` |
| Launcher browser tests | `launcher-ui/tests/e2e/*.spec.ts` (Playwright, against the mock backend). Screenshots in `visual.spec.ts-snapshots/`, dark and light at 1280 px | `npm run e2e` in `launcher-ui` |
| Legacy app | `ui/tests/`: JavaScript tests that read `ui/app.js` | `node ui/tests/run.js` |
| Mods | `../MnB-Mods/tests/` | `python tests/run_luajit.py tests/<name>_test.lua` from `MnB-Mods` |

Everything under `cargo test --workspace` is the first four rows.

## Files that tests write, and how to refresh them

| File | Refreshed with | When |
|---|---|---|
| `crates/qm-core/tests/fixtures/**` golden outputs | `QM_UPDATE_GOLDENS=1 cargo test -p qm-core --test suite golden::` | QuarterMaster's own start-up Lua changed, or patch output changed on purpose. Check that only the lines you expect differ |
| `launcher-ui/src/lib/bindings.ts` | `UPDATE_BINDINGS=1 cargo test -p qm-launcher --test suite bindings::` | A type the launcher receives changed |
| `schemas/*.json` | `qm schema --out schemas` | The mod.json, qm.toml or faction.toml schema changed |
| `launcher-ui/tests/e2e/visual.spec.ts-snapshots/*.png` | `npx playwright test tests/e2e/visual.spec.ts -g "<name>" --update-snapshots` | A screen changed on purpose. Look at the new image before keeping it |

Generated and safe to delete at any time: `target/`, `launcher-ui/dist-e2e/`, `launcher-ui/test-results/`, `launcher-ui/playwright-report/`, and `__pycache__` folders.

## Things to know

- Each crate's integration tests are one program, so every test of a crate shares one process and they run several at a time. A test that changes something the whole process sees, such as an environment variable, must take `support::exclusive()` first (qm-service): it then runs alone, and the variables are removed when it finishes. Every other test is kept apart from it by its `Fixture`.
- A helper that two areas need goes in the crate's `common` or `support` module, not into both files.
- `crates/qm-core/testdata/faction_harness.lua` is outside `tests/` because a unit test uses it, and `tests/` is not in the repository.
- Tests never launch `mnb.exe`.
