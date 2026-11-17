<h1 align="center">Agent Panel</h1>

<p align="center">
  <strong>A desktop GUI for your coding agents, based on Monocode Native.</strong>
</p>

<p align="center">
  <img width="1680" height="1050" alt="Monocode with agent sessions open side by side" src="https://github.com/user-attachments/assets/89e04066-69de-473d-9aab-88b9ca3e3b7b" />
  <br />
  <sub>Screenshot of the original Monocode app.</sub>
</p>

Agent Panel is a fork of [Monocode](https://github.com/hardbeat920/monocode) by hardbeat920. It keeps only the Rust + GPUI desktop app (Monocode Native) and adds its own changes on top, most of them for Claude Code sessions.

It runs the agent CLIs you already use: Claude Code, Codex, Cursor, Grok Build, OpenCode, Antigravity, Pi, omp, fx, Hermes Agent and Devin. If they're installed and logged in, Agent Panel can run them. Tabs are sessions, and the composer is the input. Agent Panel does not sell tokens.

## Original project

Agent Panel is based on **Monocode**: <https://github.com/hardbeat920/monocode>

Monocode's releases, documentation and issue tracker live there. Report problems with Agent Panel's own changes in this repository.

## What Agent Panel adds

- Claude sessions get two views in the Explorer. **Agents** lists the agents a chat started. **In this chat** shows the to-do list, MCP servers, tools, files, context files and skills Claude used.
- Each split pane can open its own project panel, with its own tab, width and Git status.
- A folder box at the top of the Explorer shows any folder you type or pick, in place of the chat's own folder.
- Opening the app again adds a window to the running app. Windows share live sessions, so a Claude CLI runs once however many windows show it.
- The Claude Code CLI view follows `/clear` and `/resume`, so the tab stays on the conversation the CLI moved to.
- The chevron next to the new-tab button starts a session on a model you pick.
- Chat backgrounds have a blur slider.
- The window can be translucent on Linux when your compositor supports it.
- Terminal programs that track the mouse receive clicks and drags.
- Generated commit messages are shorter: one subject line of 50 characters or less, with a body only when a change has several parts.
- Claude sessions start with Claude Code's to-do tools on (`CLAUDE_CODE_ENABLE_TODO_TOOLS=1`) unless you set that variable yourself.

## Install

There are no prebuilt releases of Agent Panel yet, so build it from source (below). The original Monocode Native preview builds are on the [Monocode releases page](https://github.com/hardbeat920/monocode/releases); they don't include Agent Panel's changes.

Install and sign in to at least one supported agent first. See [provider setup](docs/providers.md).

> [!WARNING]
> Agent Panel uses your existing Monocode database and saves changes there (on Linux, `~/.local/share/com.monocode.desktop/`). Back up that folder with the app closed before you try it, and don't edit the same conversations in Monocode and Agent Panel at once.

## Build from source

You need a stable Rust toolchain. On macOS you also need Xcode. On Debian or Ubuntu, install the build packages first:

```sh
sudo apt-get install clang cmake ninja-build pkg-config libfontconfig1-dev \
  libfreetype6-dev libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxcb-randr0-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libasound2-dev libx11-xcb-dev libegl1-mesa-dev libvulkan-dev libdbus-1-dev
```

Then build the app:

```sh
cargo build --manifest-path crates/monocode-gpui/Cargo.toml --locked -p monocode-gpui --release
```

The app is written to `crates/monocode-gpui/target/release/monocode-gpui`.

With Node.js 22+ you can also use the npm scripts:

```sh
npm run native:dev     # launch with a separate development database
npm run native:build   # build an optimized app that uses your installed Monocode data
```

Type `r` then Enter in the development terminal to rebuild and restart; there is no hot reload. The [native guide](docs/native/README.md) covers prerequisites, build output and checks, and [known limitations](docs/native/parity.md) lists what is still missing.

The mobile companion app is a separate Expo project in [`mobile/`](mobile/README.md). The `host/` folder holds the service that runs [remote sessions](docs/remote-access.md) on another machine.

## Guides

- [Provider setup](docs/providers.md)
- [Native guide](docs/native/README.md)
- [Agent access with /operator](docs/agent-access.md)
- [Remote sessions, experimental](docs/remote-access.md)

## Contributing

Bug reports and small, focused pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening one.

## Credits

Agent Panel is built on Monocode by hardbeat920 and the Monocode contributors.

[![Monocode contributors](https://contrib.rocks/image?repo=hardbeat920/monocode)](https://github.com/hardbeat920/monocode/graphs/contributors)

Contributor image by [contrib.rocks](https://contrib.rocks). Third-party code, icons and their licences are listed in [NOTICE](NOTICE).

## License

[MIT](LICENSE), the same licence as Monocode. Provider names and logos are trademarks of their owners. See [NOTICE](NOTICE).
