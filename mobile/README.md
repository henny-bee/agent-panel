# MCode mobile

The mobile companion comes from Monocode. MCode builds it for Android under its own name and app ID (`com.hennybee.mcode`); the iOS settings are still Monocode's.

The Expo app lives in this directory and has its own npm dependencies and lockfile. The app pairs with the desktop over QUIC and shows its real workspace. Use a Node.js LTS release for development.

## Run locally

From the repository root:

```bash
cd mobile
npm ci
npm run ios
```

`npm run ios` first builds the Rust connection library, then creates and installs the development build in an iOS simulator, then starts Metro. It requires Xcode 27.1 or newer (including the iOS 27.1 SDK for hinge observation) and an installed iOS Simulator runtime (Xcode > Settings > Components). For Android, install Android Studio and start an emulator, install an Android NDK, set `ANDROID_NDK_HOME`, then use `npm run android`. Rust and rustup must be installed for both platforms. Use `npm run ios -- --device` or `npm run android -- --device` to build for a connected physical device. A physical iPhone needs Developer Mode enabled and a unique `ios.bundleIdentifier` in `app.json`.

After the first build, use `npm start` for everyday TypeScript and JavaScript changes. Press `i` or `a` in the Expo terminal to open the installed development build in a simulator or emulator. Rebuild after adding native modules, changing `app.json`, or upgrading Expo. Regenerate native folders with `npx expo prebuild --clean` before rebuilding in those cases. The generated `ios/` and `android/` folders are ignored by Git.

The routes are in `src/app/`:

- `_layout.tsx` gates the workspace behind pairing and keeps one navigation stack inside `WorkspaceLayout`. A projects/recent-sessions sidebar appears when the container has at least 740 points of usable width; narrower containers keep compact navigation. This width-based behavior also applies to tablets and wide Android windows.
- `project/[projectId]/index.tsx` is the new session screen for a project.
- `project/[projectId]/session/[sessionId].tsx` is a session transcript.
- `mono/[monoId].tsx` is a mono's conversation.

On iPhone Duo running iOS 27.1 or newer, folding in portrait to an opening of about 120° or less focuses the visible chat composer. A slight bend does not trigger it; opening past 150° rearms it. Dismissing the keyboard keeps it dismissed until you leave and re-enter that posture. Folding while the composer is unavailable or a sheet/menu is presented does not defer focus until later. Other devices keep manual keyboard focus. In Device Hub, disable **Device → Keyboard → Simulate Hardware Keyboard** to test the onscreen keyboard.

The sidebar and pane resizing preserve the mounted conversation and draft. Native transcript scroll anchoring still needs simulator/device validation; mocked React tests establish component identity rather than native scroll offsets.

## Pair with desktop

Run the desktop app from the repository root, open **Settings → Mobile**, enable mobile access and choose **Pair device**. Scan the code in the mobile app, then choose the projects and monos to include. You can select nothing and choose later. **Mobile Settings → Projects and monos** changes the selection at any time; choices are remembered per desktop, including after reconnecting.

Pairing codes expire after 15 minutes and work once. The shared Rust module uses QUIC and mutually authenticated TLS 1.3, with the desktop identity pinned by the code. Long-lived private keys are stored in the desktop OS keyring and the phone’s SecureStore. Desktop settings list paired devices and let you revoke them. Selection controls what appears on the phone; a paired device has workspace access.

Remote connections go directly to the desktop. Automatic NAT-PMP/UPnP router mapping is enabled when mobile access is first enabled. A restrictive router or carrier NAT can require a reachable VPN address or manual UDP forwarding to port **38416**. Configure the remote address before generating a code. The desktop must remain awake and running.

Workspace lists refresh while the phone is active. Opening a conversation reads its latest 200 user/assistant blocks from the desktop, with active conversation updates, text sending, starting sessions and stopping turns. The changes view reads real Git changes on demand. Agent approvals and questions still need attention on the desktop. The app retains the workspace list for offline navigation and keeps failed-send drafts; it does not queue commands offline.

The transport is an Expo local native module in `modules/monocode-link`, built by `scripts/build-native-link.mjs`. **Expo Go cannot load it**; rebuild the development app after changing native code. `npm run native:ios` and `npm run native:android` build the library separately. Generated native libraries and platform folders are ignored by Git.

## Switch provider accounts

Tap the model in the composer, then **Account**, to choose a Claude Code or Codex profile saved on your desktop. Each provider keeps its own selection. Choosing another account from an existing chat opens a new conversation with the same model and permissions; the original stays in history. Add accounts and complete sign-in in desktop **Settings → Accounts**.

The account card also shows the desktop footer's usage windows, percentages and reset times. Tap **Refresh** or pull down in the model picker to reload usage for the provider's accounts.

## EAS development builds

Run EAS commands from `mobile/`. The iOS development and production profiles pin the Xcode 27.1 build image in `eas.json`; the simulator profile inherits it. Changes to the native transcript or hinge observer require a new native build, rather than only an EAS Update. The build input must include the root Cargo workspace and its path dependencies. The `eas-build-pre-install` hook installs the pinned Rust toolchain and builds the native transport before CocoaPods/Gradle runs.

The `development` profile builds for Android or physical iOS devices; `development-simulator` builds for the iOS simulator. EAS requires an Expo account, and physical iOS distribution requires Apple signing credentials.

```bash
npx eas-cli@latest login
npx eas-cli@latest build --platform ios --profile development-simulator
# or: npx eas-cli@latest build --platform android --profile development
```

Install the completed development build, then run `npm start` to connect it to Metro.

## Build an Android APK

This works on Windows, macOS and Linux without EAS. It needs Rust, an Android SDK with an NDK in `ANDROID_HOME` and a JDK 17 in `JAVA_HOME`.

```bash
cd mobile
npm ci
npm run native:android
npx expo prebuild -p android
cd android
./gradlew assembleRelease -PreactNativeArchitectures=arm64-v8a,x86_64
```

On Windows, a deep checkout can fail with `ninja: error: manifest 'build.ninja' still dirty after 100 tries`. The default CMake 3.22.1 ships ninja 1.10, which can't read paths over 260 characters. Install a newer CMake with `sdkmanager "cmake;3.31.6"`, add `cmake.dir=C\:/Users/<you>/AppData/Local/Android/Sdk/cmake/3.31.6` to `android/local.properties`, and build again. `expo prebuild --clean` deletes that file, so add the line again afterwards.

The APK is `android/app/build/outputs/apk/release/app-release.apk`. Install it with `adb install` or copy it to the phone. Leave out `-PreactNativeArchitectures` to also include the 32-bit ABIs. The release build is signed with the debug key, which is fine for installing directly but not for Google Play.

The app has no EAS project, so it doesn't receive over-the-air updates or Expo push notifications. Both need an Expo project of your own in `app.json`.

## Check the project

```bash
npx tsc --noEmit
npm run lint
```

The desktop app at the repository root remains a separate npm project.
