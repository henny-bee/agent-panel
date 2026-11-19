# Build from source

MCode is the Rust + GPUI desktop app in `crates/monocode-gpui`. It builds on Windows, macOS and Linux. Monocode's older React/Tauri app is not part of MCode, so Monocode's Tauri build commands don't apply here.

## What you need

- A current stable Rust toolchain from [rustup](https://rustup.rs).
- Node.js 22+, only if you want the npm scripts below. Plain `cargo` works without it.
- At least one agent CLI, installed and signed in. See [provider setup](providers.md).

Each platform also needs the following:

| Platform | Also install |
| --- | --- |
| Windows | The `stable-x86_64-pc-windows-msvc` toolchain and the Visual Studio Build Tools with the **Desktop development with C++** workload (linker and Windows SDK) |
| macOS | Xcode with its macOS components |
| Debian / Ubuntu | The packages below |

```sh
sudo apt-get install clang cmake ninja-build pkg-config libfontconfig1-dev \
  libfreetype6-dev libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxcb-randr0-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libasound2-dev libx11-xcb-dev libegl1-mesa-dev libvulkan-dev libdbus-1-dev
```

## Build the app

From the repository root:

```sh
cargo build --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-gpui --release
```

| Platform | The app |
| --- | --- |
| Windows | `crates\monocode-gpui\target\release\monocode-gpui.exe` |
| macOS and Linux | `crates/monocode-gpui/target/release/monocode-gpui` |

`npm run native:build` does the same. On macOS it also wraps the app in a bundle that is still named `Monocode Native.app`:

```sh
open "crates/monocode-gpui/target/release/Monocode Native.app"
```

A release build uses your existing Monocode data. See [install](install.md#where-mcode-keeps-its-data) before you open it for the first time.

## Develop

```sh
npm run native:dev
npm run native:dev -- --cwd /path/to/project   # open a particular project
```

A development build uses its own database, so your real conversations stay untouched. Type `r` then Enter in that terminal to rebuild and restart. The running app stays open until the new build succeeds. Ctrl+C stops the app and the runner. There is no hot reload.

## Check

Run the same checks as CI before you push:

```sh
npm run native:check
```

It runs the script tests, `cargo fmt --check`, `cargo clippy` with warnings as errors, and the tests for both crates. To run one step on its own:

```sh
cargo fmt --manifest-path crates/monocode-gpui/Cargo.toml --all
cargo clippy --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-core -p monocode-gpui --all-targets --profile native-dev -- -D warnings
cargo test --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-core --profile native-dev
cargo test --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-gpui --profile native-dev -- --test-threads=1
```

The app's tests must run one at a time (`--test-threads=1`). CI runs them on macOS. On Windows and Linux, a few UI tests press macOS shortcuts or expect Unix paths and fail, so compare against `main` there.

## Update a build

Builds made from source don't update themselves. Pull the latest code and build again. On Windows, close MCode first: Windows doesn't let a build replace a program that is running.

## Packages

MCode doesn't publish packages yet. The repository still has Monocode's packaging scripts (`scripts/package-native.mjs`, `scripts/native-installer.nsi`, `scripts/bundle-native-macos.sh`) and its release workflows. They still use the Monocode Native name and haven't been tried for MCode.

The remote-session host has its own [build and packaging steps](remote-access.md#release-packaging).

See the [native guide](native/README.md) for data paths, fixtures and the remaining details.
