# Install MCode

MCode has no prebuilt releases yet. Build it from source as described in [building](building.md), then run the app it produces:

| Platform | Run |
| --- | --- |
| Windows | `crates\monocode-gpui\target\release\monocode-gpui.exe`. You can pin it to the taskbar. |
| macOS | `crates/monocode-gpui/target/release/Monocode Native.app`, made by `npm run native:build` |
| Linux | `crates/monocode-gpui/target/release/monocode-gpui` |

Install and sign in to at least one [agent CLI](providers.md) before you start a session.

Monocode's own `.deb`, `.rpm`, AppImage and installer packages are on the [Monocode releases page](https://github.com/hardbeat920/monocode/releases). They are Monocode, not MCode, and don't include MCode's changes.

## Where MCode keeps its data

A release build opens Monocode's database and saves changes there, so you see the same projects and conversations as in Monocode.

| Platform | Data folder |
| --- | --- |
| Windows | `%APPDATA%\com.monocode.desktop\` |
| macOS | `~/Library/Application Support/com.monocode.desktop/` |
| Linux | `$XDG_DATA_HOME/com.monocode.desktop/`, or `~/.local/share/com.monocode.desktop/` |

> [!WARNING]
> Back up that folder with the app closed before you open MCode for the first time. Don't edit the same conversations in Monocode and MCode at the same time.

MCode's own preferences are saved next to the database in `monocode.db.gpui.sqlite3`.

A development build (`npm run native:dev`) uses a separate `Monocode Development` folder instead. The [native guide](native/README.md#data-and-configuration) lists where it is on each platform. Set `MONOCODE_NATIVE_DATA_DIR` to use another folder.

To check the database without opening a window or changing anything:

```sh
crates/monocode-gpui/target/release/monocode-gpui --check-data
```

## Updates

MCode doesn't update itself: its update check says automatic updates aren't configured for this build. To update, pull the latest code and build again. On Windows, close MCode first, because a running program can't be replaced.

## Uninstall

Delete the repository folder or its `crates/monocode-gpui/target` folder. Your data folder stays, because Monocode uses it too. Delete it yourself only if you no longer use Monocode either.
