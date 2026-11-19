# Native architecture

The GPUI app began in Monocode as a preview alongside its React/Tauri app, which
served as its behavior reference. MCode keeps only the GPUI app. What remains of
the React code in `src/` is used only by the asset generators in `scripts/`.

## Code layout

| Location | Responsibility |
| --- | --- |
| `crates/monocode-core/` | Provider transports, persistence, accounts, Git/files, remote connections and shared services |
| `crates/monocode-gpui/` | Native shell, composer, transcript, editor, settings and platform UI |
| `crates/monocode-gpui/Cargo.toml` | Independent native Cargo workspace, profiles and dependency patches |
| `scripts/dev-native.mjs` | Development build and manual restart runner |
| `scripts/build-native.mjs` | Optimized build and macOS packaging |
| `scripts/native-editor/`, `scripts/native-settings/` | Generators for embedded JavaScript assets |
| `src/` | TypeScript from Monocode's React app, still imported by those generators |
| `vendor/gpui-pre*/` | Pinned framework and platform patches |

`monocode-core` has no Tauri or GPUI dependency. Keep provider and storage
behavior in core services, with UI work in GPUI views. Development uses isolated
data; release builds use the installed desktop database. See the
[native guide](README.md#data-and-configuration) for paths and commands.

Provider transports use Codex app-server, Claude stream-json, OpenCode HTTP/SSE,
ACP for Cursor/Grok/Antigravity/fx/Hermes/Devin, and Pi/omp RPC. They reuse the
providers' CLI authentication and conversation IDs.

## Rendering and responsiveness

Keep provider I/O, parsing, file access and persistence off the UI thread.
Composer input should not rebuild the transcript. Transcript rows have stable
identities, bounded blocks and virtualized layout; streaming updates should
preserve the reader's position. Cancellation must remain responsive under load.

The editor embeds generated analysis and formatting bundles. After changing their
source services or dependencies, run `npm ci` and `npm run build` inside
`scripts/native-editor/`. Generated bundles and their license notices belong
in the repository.

## Framework patches

GPUI and its platform crate are pinned to `gpui-pre` 0.3.9. Local Apple renderer
patches cover alpha blending, font matching, backdrop blur and shadows. The
headless Metal regression is documented in [verification](verification.md).

The vendored framework/platform crates remove the unused `zed-scap` dependency.
The native build rejects lockfiles that reintroduce it, and the `screen-capture`
feature remains disabled. Preserve these guards and the licenses in [NOTICE](../../NOTICE).

See [preview limitations](parity.md) for current release work. Tauri remains the
production app until an explicit cutover.
