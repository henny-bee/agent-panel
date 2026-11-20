# Contributing

MCode is a small fork of [Monocode](https://github.com/hardbeat920/monocode) with one maintainer. Bug reports and small, focused pull requests are welcome. For a bigger change, open an issue first so we can agree on the scope before you write it.

If a problem also happens in the original Monocode, please report it there too: <https://github.com/hardbeat920/monocode/issues>.

## Get it running

You need a stable Rust toolchain and at least one agent CLI installed and logged in. See [provider setup](docs/providers.md). The README lists [what each platform needs](README.md#build-from-source): the Visual Studio C++ build tools on Windows, Xcode on macOS, and a list of packages on Linux.

With Node.js 22+, start a development build that uses its own database, so your real conversations stay untouched:

```sh
npm run native:dev
```

Type `r` then Enter in that terminal to rebuild and restart.

## Where things live

- `crates/monocode-core/` holds the agent providers, sessions, storage and Git code. It has no UI.
- `crates/monocode-gpui/` is the desktop app, built with GPUI. It has its own Cargo workspace and lockfile.
- `vendor/gpui-pre*/` is the vendored GPUI framework, with the changes listed in [NOTICE](NOTICE).
- `mobile/` is the mobile companion app.
- `src/` is TypeScript from Monocode's old React app. The app doesn't run it, but the editor and Settings arcade generators in `scripts/` build from it.
- `docs/native/` has the [native guide](docs/native/README.md) and its [architecture notes](docs/native/migration.md).

## Before you push

Run the same checks as CI:

```sh
npm run native:check
```

That runs the script tests, `cargo fmt --check`, `cargo clippy` with warnings as errors, and the tests for both crates. To run one step on its own:

```sh
cargo fmt --manifest-path crates/monocode-gpui/Cargo.toml --all
cargo clippy --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-core -p monocode-gpui --all-targets --profile native-dev -- -D warnings
cargo test --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-core --profile native-dev
cargo test --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-gpui --profile native-dev -- --test-threads=1
```

CI runs clippy and the tests for the app on macOS. On Linux and Windows, some UI tests press macOS shortcuts or expect Unix paths and fail, and on Linux clippy reports some macOS-only code as unused. There, compare against `main` instead of expecting a clean run.

## Pull requests

Keep a pull request to one change, and say what changed and why. The [PR template](.github/pull_request_template.md) covers the rest. If it changes the UI, add a before and after screenshot.

Write short commit messages as `type(area): what it does now`, such as `feat(chat): add blur slider for chat backgrounds`. Add a few bullets under it only when a change has several parts. Name branches `type/short-desc`, such as `fix/12-graph-after-switch`.

I may ask you to shrink a pull request, or close it if it doesn't fit the project. That's about scope and timing, not about you or the quality of your work.

Be kind: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Security reports: [SECURITY.md](SECURITY.md).
