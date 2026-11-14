# D-Bus interface: `com.ca-peterson.RclonePlasma`

This is the single contract between the Rust daemon (`rclone-plasma --daemon`), the QML plasmoid, the cxx-qt windows, and the C++ Dolphin plugins. Change it here first.

- Bus: **session**
- Service name: `com.ca-peterson.RclonePlasma`. It is D-Bus-activated through `dbus/com.ca-peterson.RclonePlasma.service`, whose `Exec` is `rclone-plasma --daemon`.
- Object path: `/com/ca_peterson/RclonePlasma`
- Interface: `com.ca_peterson.RclonePlasma1`

## Properties (read-only; changes are announced with `org.freedesktop.DBus.Properties.PropertiesChanged`)

| Name | Type | Meaning |
|---|---|---|
| `Health` | `s` | One of the health names below. |
| `StatusJson` | `s` | The full snapshot. Schema below. |
| `MountPath` | `s` | Absolute, `~`-expanded mount path, with no trailing slash. |

`Health` values, in `rc::Health` order: `ServiceStopped`, `NotMounted`, `Unreachable`, `CacheFull`, `FileErrors`, `Uploading`, `Downloading`, `Transferring`, `Healthy`. Before the first poll it is `Loading`.

Each value maps to an installed icon named `rclone-plasma-<kebab>`: `rclone-plasma-stopped`, `-unmounted`, `-unreachable`, `-full`, `-error`, `-uploading`, `-downloading`, `-transferring`, `-synced`, and `-loading`. These come from upstream `resources/emblems/*.svg`.

## Methods

| Signature | Meaning |
|---|---|
| `ServiceAction(s verb) -> b` | `verb` ∈ `restart`, `stop`, `start` (runs `systemctl --user <verb> <service>`). |
| `OpenFolder()` | Opens the mount path with `xdg-open`. |
| `OpenDetails(s path)` | Spawns `rclone-plasma --menu <path>`. `path` is an absolute local path, or remote-relative if it doesn't start with `/`. |
| `OpenSettings()` | Spawns `rclone-plasma --settings`. |
| `FileStates(as paths) -> a{sas}` | Maps each absolute local path to emblem icon names (see the Overlay emblems section below). A path outside the mount maps to `[]`. This must answer quickly from a cache: a cache miss returns `[]` straight away and computes the state in the background. When the state is ready, the daemon emits `ItemsChanged`. |
| `GetConfig() -> s` | The config as JSON (fields from `rclone_ui_core::config::Config`). |
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
  "transfers": [
    {"name": "Docs/a.pdf", "size": 1000, "bytes": 500, "percentage": 50, "speed": 1024.0,
     "direction": "Upload", "mimeIcon": "application-pdf"}
  ],
  "recent": [
    {"name": "Docs/b.txt", "size": 10, "at": "2026-11-14T12:00:00Z", "atText": "12:00",
     "error": null, "direction": "Download", "times": 2, "kind": "Updated", "mimeIcon": "text-plain"}
  ],
  "offline": {
    "enabled": true,
    "pins": [{"rel": "Projects", "phase": "Ready", "phaseLabel": "Available offline",
              "phaseIcon": "emblem-ok-symbolic", "error": null}]
  }
}
```

Notes on the fields:
- `recent` is newest first, deduplicated (with `times`), and capped at 100. The daemon applies the 2 s debounce, so the UI shows it as-is.
- `direction` is `"Upload"`, `"Download"`, or `null`.
- `kind` is `"Created"`, `"Updated"`, `"Deleted"`, or `null`.
- `mimeIcon` is a freedesktop icon name chosen from the file name only.

## Overlay emblems (`FileStates`)

Each path maps from `status::SyncState` to emblems as follows:

| SyncState | Emblem |
|---|---|
| InSync | `rclone-plasma-emblem-synced` |
| OnlineOnly | `rclone-plasma-emblem-online` |
| PartlyCached | `rclone-plasma-emblem-partial` |
| Uploading, QueuedUpload, LocalNewer | `rclone-plasma-emblem-uploading` |
| Downloading, DriveNewer | `rclone-plasma-emblem-downloading` |
| Differs | `emblem-warning` |

Pinned offline folders, and every path inside them, also get `rclone-plasma-emblem-offline`. A pin in phase `Attention` gets `emblem-error` instead.
