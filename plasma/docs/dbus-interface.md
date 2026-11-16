# D-Bus interface: `com.ca-peterson.rclone-shell-ext`

This is the single contract between the Rust daemon (`rclone-shell-ext-plasma --daemon`), the QML plasmoid, the cxx-qt windows, and the C++ Dolphin plugins. Change it here first.

- Bus: **session**
- Service name: `com.ca-peterson.rclone-shell-ext`. It is D-Bus-activated through `dbus/com.ca-peterson.rclone-shell-ext.service`, whose `Exec` is `rclone-shell-ext-plasma --daemon`.
- Object path: `/com/ca_peterson/rclone_shell_ext`
- Interface: `com.ca_peterson.RcloneShellExt1`

## Properties (read-only; changes are announced with `org.freedesktop.DBus.Properties.PropertiesChanged`)

| Name | Type | Meaning |
|---|---|---|
| `Health` | `s` | One of the health names below. |
| `StatusJson` | `s` | The full snapshot. Schema below. |
| `MountPath` | `s` | Absolute, `~`-expanded mount path, with no trailing slash. |

`Health` values, in `rc::Health` order: `ServiceStopped`, `NotMounted`, `Unreachable`, `CacheFull`, `FileErrors`, `Uploading`, `Downloading`, `Transferring`, `Healthy`. Before the first poll it is `Loading`.

Each value maps to an installed icon named `rclone-shell-ext-<kebab>`: `rclone-shell-ext-stopped`, `-unmounted`, `-unreachable`, `-full`, `-error`, `-uploading`, `-downloading`, `-transferring`, `-synced`, and `-loading`. These are installed from `resources/emblems/<kebab>.svg`.

## Methods

| Signature | Meaning |
|---|---|
| `ServiceAction(s verb) -> b` | `verb` ∈ `restart`, `stop`, `start` (runs `systemctl --user <verb> <service>`). |
| `OpenFolder()` | Opens the mount path with `xdg-open`. |
| `OpenDetails(s path)` | Spawns `rclone-shell-ext-plasma --menu <path>`. `path` is an absolute local path, or remote-relative if it doesn't start with `/`. |
| `OpenSettings()` | Spawns `rclone-shell-ext-plasma --settings`. |
| `FileStates(as paths) -> a{sas}` | Maps each absolute local path to emblem icon names (see the Overlay emblems section below). A path outside the mount maps to `[]`. This must answer quickly from a cache: a cache miss returns `[]` straight away and computes the state in the background. When the state is ready, the daemon emits `ItemsChanged`. |
| `Strings() -> a{ss}` | Every UI message that has no variables (id → text), in the daemon's language (`rclone_ui_i18n::static_messages()`). The plasmoid and the Dolphin plugins use it for all their wording; ids that need arguments never appear here and arrive preformatted in `StatusJson`. Call it once and cache it. Being a method call, it also D-Bus-activates the daemon. |
| `GetConfig() -> s` | The config as JSON (fields from `rclone_ui_core::config::Config`). If the config file exists but can't be read (corrupt), it is instead `{"errorText": "<finished, translated message including the reason>"}`: show `errorText` as is, and don't offer to edit. |
| `SetConfig(s json) -> s` | Merges these JSON fields into the config, saves it, and returns `""` on success or an error message. |

## Signals

| Signature | Meaning |
|---|---|
| `ItemsChanged(as paths)` | The overlay emblems of these absolute paths may have changed. The overlay plugin re-queries any it is showing. |

## `StatusJson` schema

```json
{
  "health": "Uploading",
  "healthLabel": "Uploading 3 files",
  "serviceActive": true,
  "mounted": true,
  "rcError": null,
  "uploadsInProgress": 1,
  "uploadsQueued": 2,
  "cacheFiles": 120,
  "cacheBytes": 123456789,
  "erroredFiles": 0,
  "outOfSpace": false,
  "speed": 1048576.0,
  "speedText": "1.0 MiB/s",
  "lastError": null,
  "rcloneVersion": "v1.75.2",
  "outdatedRclone": null,
  "outdatedText": null,
  "lastErrorText": null,
  "emptyHistoryText": "Nothing synced since rclone started",
  "showAllText": "Show all (1)",
  "transfers": [
    {"name": "Docs/a.pdf", "size": 1000, "bytes": 500, "percentage": 50, "speed": 1024.0,
     "direction": "Upload", "mimeIcon": "application-pdf",
     "line": "Docs/a.pdf — 500.0 B / 1000.0 B at 1.0 KiB/s"}
  ],
  "recent": [
    {"name": "Docs/b.txt", "size": 10, "at": "2026-11-14T12:00:00Z", "atText": "12:00",
     "error": null, "direction": "Download", "times": 2, "kind": "Updated", "mimeIcon": "text-plain",
     "caption": "Docs · 10 B · 5 minutes ago · ×2", "errorText": null,
     "directionTip": "Downloaded from the cloud", "kindIcon": "document-edit-symbolic",
     "kindLabel": "Changed"}
  ],
  "offline": {
    "enabled": true,
    "line": "Offline: 1 folder · all available offline",
    "pins": [{"rel": "Projects", "phase": "Ready", "phaseLabel": "Available offline",
              "phaseIcon": "emblem-ok-symbolic", "error": null, "detail": "Available offline"}]
  },
  "migration": "none",
  "migrationLabel": ""
}
```

Notes on the fields:
- `recent` is newest first, deduplicated (with `times`), and capped at 100. The daemon applies the 2 s debounce, so the UI shows it as-is.
- `direction` is `"Upload"`, `"Download"`, or `null`.
- `kind` is `"Created"`, `"Updated"`, `"Deleted"`, or `null`.
- `mimeIcon` is a freedesktop icon name chosen from the file name only.
- Fields named `health`, `phase`, `direction`, `kind` and `outdatedRclone` are machine values (codes); show the neighbouring finished text instead: `healthLabel`, `phaseLabel`/`detail`, `directionTip`, `kindLabel`/`kindIcon`, `outdatedText`. Never recompose text from the codes.
- `outdatedText`, `lastErrorText`, `errorText` and `directionTip` are `null` when there is nothing to say. `emptyHistoryText` explains an empty `recent`. `showAllText` is the label of the button that expands the history.
- `transfers[].line` and `recent[].caption` are finished one-line descriptions (name/progress/speed, and folder/size/age/count). `kindIcon` is a freedesktop icon name.
- `offline.line` is the one-line offline summary (or a nudge to finish setup); `pins[].detail` is `phaseLabel` followed by the error when there is one.
- Wording that needs no arguments (button labels, headings) comes from `Strings()`, not from here.
- `migration` is the state of the one-time move from the old `rclone-plasma` / `cosmic-rclone-ui` installs (`core::migrate::pending`). It is a code: `none` (nothing to do), `pending` (an old install is waiting; the daemon is moving it in the background), or the reason the last attempt failed: `ambiguous`, `conflict`, `sync-in-flight`, `mount-in-use`, `lock-timeout`, `failed`. While it is not `none`, offline folders can't be turned on or pinned. `migrationLabel` is the finished, translated text for it (empty when `none`); show it as a banner. The daemon starts the migration on a background thread after it owns its bus name, so activation never waits for it.

## Overlay emblems (`FileStates`)

Each path maps from `status::SyncState` to emblems as follows:

| SyncState | Emblem |
|---|---|
| InSync | `rclone-shell-ext-emblem-synced` |
| OnlineOnly | `rclone-shell-ext-emblem-online` |
| PartlyCached | `rclone-shell-ext-emblem-partial` |
| Uploading, QueuedUpload, LocalNewer | `rclone-shell-ext-emblem-uploading` |
| Downloading, DriveNewer | `rclone-shell-ext-emblem-downloading` |
| Differs | `emblem-warning` |

Pinned offline folders, and every path inside them, also get `rclone-shell-ext-emblem-offline`. A pin in phase `Attention` gets `emblem-error` instead.
