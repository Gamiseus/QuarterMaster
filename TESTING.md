# Testing QuarterMaster

Nothing here starts the game.

## Rust

```powershell
cargo test --workspace
```

This runs the unit tests of every crate. One crate at a time: `cargo test -p <crate> --lib`.

Some engine tests run QuarterMaster's Lua on the game's own `lua51.dll`. They are skipped when the game is not found, so the command passes on a machine without it.

## Launcher

```powershell
cd launcher-ui
npm install
npm run check
```

`npm run check` type-checks the launcher. To try it without the game, `npm run dev:mock` serves it in a browser against sample data.

## What is not in the repository

The integration tests and the launcher's own test suites are not published, so `cargo test` and `npm run check` are what a clone can run. `check.ps1` is the full release gate and expects those suites; it stops where they are missing.

## Checking a mod

To test a mod rather than QuarterMaster, use `qm lint <mod>` and `qm validate`. The [modding wiki](https://gamiseus.github.io/wiki/) covers both under testing and debugging.
