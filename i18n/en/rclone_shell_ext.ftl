# User-visible text for every surface. One line per message: Fluent keeps line breaks in
# multi-line values, and the wording must match what the UI showed before it was externalized.

## common-: short words used in several places

common-yes = Yes
common-no = No
common-continue = Continue
common-ago-now = just now
common-ago-minutes = { $count ->
    [one] 1 minute ago
   *[other] { $count } minutes ago
}
common-ago-hours = { $count ->
    [one] 1 hour ago
   *[other] { $count } hours ago
}
common-ago-days = { $count ->
    [one] 1 day ago
   *[other] { $count } days ago
}

## details-: the details window

details-status-error = Error
details-status-checking = Checking…
details-status-cached-no-compare = Cached (can't compare with Drive)
details-status-transferring = Transferring
details-status-nothing-pending = Nothing pending
details-files-pending = { $count } file(s)
details-files-pending-more = { $count } or more file(s)
details-files-size = { $count } file(s), { $size }
details-files-size-more = { $count }+ file(s), { $size }
details-cached-partly = Partly ({ $have } of { $size })

details-state-uploading = Uploading
details-state-queued-upload = Queued for upload
details-state-downloading = Downloading
details-state-in-sync = In sync (cached)
details-state-online-only = Online only (not cached)
details-state-partly-cached = Partly downloaded
details-state-local-newer = Local changes pending
details-state-drive-newer = Drive has a newer version
details-state-differs = Differs from Drive

details-row-uploading = Uploading
details-row-downloading = Downloading
details-row-offline-copy = Offline copy
details-row-cached = Cached
details-row-drive-modified = Drive modified
details-row-cached-locally = Cached locally
details-row-size = Size
details-row-version = Version

details-sharing-unavailable = Unavailable
details-sharing-not-shared = Not shared
details-sharing-shared-with = Shared with { $count }
details-sharing-error = Couldn't load sharing info: { $error }
details-sharing-only-you = Only you have access
details-sharing-unlisted = Google Drive didn't list who has access

details-manage-sharing = Manage Sharing
details-manage-sharing-provider = Manage Sharing ({ $provider })

details-keep-offline = Keep offline
details-keep-offline-hint = Download this folder and keep it in sync both ways
details-offline-part-of = Part of the offline folder { $folder } · { $state }
details-offline-so-far = { $count } file(s), { $size } so far
details-offline-needs-attention = Offline sync needs attention
details-offline-syncing = Syncing offline copy
details-offline-waiting = Waiting to sync offline copy

## settings-: the settings window

settings-offline-description = Folders you choose are fully downloaded, kept in sync both ways, and work without a connection, right where they are in the Drive folder.
settings-offline-description-locked = Stop keeping every folder offline before turning this off.
settings-consent-continue-password = Continue to password prompt
settings-setup-hint-busy = Enter your password if asked. The Drive mount restarts once.
settings-setup-hint-remaining = Runs the remaining steps above.
settings-setup-hint-mount-down = The Drive mount isn't running; setup will start it.
settings-pin-synced = { $label } · synced { $when }

## offline-: offline folders (phases, consent, setup steps, keeper messages)

offline-phase-downloading = Downloading for offline use
offline-phase-ready = Available offline
offline-phase-waiting = Waiting to sync
offline-phase-attention = Needs attention
offline-phase-removing = Removing offline copy

offline-describe-waiting-to-start = Waiting to start…
offline-describe-so-far = { $phase } · { $files } so far
offline-describe-phase-files = { $phase } · { $files }
offline-describe-error = { $line }: { $error }
offline-syncing = Syncing…
offline-files-size = { $count ->
    [one] 1 file ({ $size })
   *[other] { $count } files ({ $size })
}
offline-folders-count = { $count ->
    [one] 1 folder
   *[other] { $count } folders
}
offline-summary-none = No folders kept offline
offline-summary-all-ready = { $folders } · all available offline
offline-summary-phase = { $folders } · { $phase ->
    [downloading] downloading for offline use
    [waiting] waiting to sync
    [attention] needs attention
    [removing] removing offline copy
   *[ready] available offline
}

offline-step-fuse-title = Allow mounts inside the Drive folder
offline-step-fuse-detail = Adds user_allow_other to /etc/fuse.conf. Asks for an administrator password once.
offline-step-units-title = Background sync service
offline-step-units-detail = Installs and starts a user service that keeps offline folders in sync.
offline-step-restart-title = Restart the Drive mount
offline-step-restart-detail = Applies --allow-other to the Drive mount. Only you can open its files. Files open in the Drive folder are interrupted once.

offline-consent-fuse-title = Change a system setting: { $path }
offline-consent-fuse-detail = Adds the line “user_allow_other”, which lets your own programs mount with --allow-other. This needs an administrator password, which you'll be asked for next. The current file is first copied to { $path }{ $infix }<date-time>.bak (if that copy fails, the file isn't touched). The line stays in place if you turn offline folders off later.
offline-consent-units-title = Install background services
offline-consent-units-detail = Adds { $keeper } and { $pin } to { $dir }, enables the first, and ties both to { $service } so they start and stop with the Drive mount.
offline-consent-dropin-title = Change the Drive mount's settings
offline-consent-dropin-detail = Adds { $path } so the mount runs with --allow-other, --default-permissions and umask 077. Files in the Drive folder then show as owner-only (0600 / 0700), which keeps other users on this computer out.
offline-consent-restart-title = Restart { $service }
offline-consent-restart-detail = Happens once, now. Anything open in { $mount } is interrupted; changes waiting to upload are kept and resume after the restart.
offline-consent-after-title = Afterwards
offline-consent-after-detail = Folders you choose to keep offline are downloaded to { $path } and use disk space there. Turning offline folders off removes the services and mount settings again.
offline-note-fuse-backup = The previous { $path } was saved as { $backup }
offline-err-not-turned-on = Offline folders weren't turned on: { $error }

offline-conflict-same = { $name } was changed the same way on both sides
offline-conflict-local = { $name } was changed in two places; kept this computer's newer version (the other is in the file's version history)
offline-conflict-remote = { $name } was changed in two places; kept the newer version from the cloud (this computer's is in the file's version history)

offline-err-outside-drive = Only items inside the Drive folder can be kept offline
offline-err-whole-drive = The whole Drive can't be kept offline; pick a folder inside it
offline-err-only-folders = Only folders can be kept offline
offline-err-already = Already kept offline
offline-err-already-part = Already kept offline as part of { $parent }
offline-err-contains = Contains { $pin }, which is already kept offline; remove that first
offline-err-path-too-long = The folder's path is too long for rclone bisync
offline-err-outside-drive-folder = Not inside the Drive folder
offline-err-turned-off = Offline folders are turned off; turn them on in the settings first
offline-err-not-offline = This folder isn't kept offline
offline-err-mount-down = The Drive mount is down

offline-err-fuse-run = Couldn't run { $program }: { $error }
offline-err-fuse-denied = Permission was not granted
offline-err-fuse-no-agent = No authentication agent is available
offline-err-fuse-backup = Couldn't back up { $path }, so it was left unchanged
offline-err-fuse-write = Couldn't write { $path }
offline-err-fuse-failed = Updating { $path } failed ({ $status })

offline-err-no-job-id = { $method }: rclone didn't return a job id
offline-err-lost-contact = lost contact with rclone during { $method }: { $error }
offline-err-earlier-sync = An earlier sync is still finishing
offline-err-folder-replaced = The folder in the cloud was deleted and recreated, so this offline copy no longer matches it. Repair merges both sides without deleting anything.
offline-err-one-side-empty = One side of the folder is empty, and syncing would empty the other side too. Repair brings the files back.
offline-err-cloud-folder-missing = The folder wasn't found in the cloud (moved or deleted?)
offline-err-sync-stopped = Sync stopped for safety (too many deletions, or an interrupted run). Repair merges both sides without deleting anything.
offline-err-mount-folder-missing = The folder wasn't found in the Drive mount (moved or deleted?)
offline-err-mount-in-place = Couldn't mount the offline copy in place: { $error }
offline-err-waiting-to-upload = Waiting for a connection to upload the last changes
offline-err-upload-last = Couldn't upload the last changes: { $error }
offline-err-files-open = Files in the folder are still open
offline-err-no-network = no network
offline-err-delete-local = Couldn't delete the local copy: { $error }
offline-err-cloud-lookup = Couldn't look up the cloud folder: { $error }
offline-notify-title = Offline folders
offline-notify-conflicts-resolved = Resolved { $count } conflicting edits in { $folder }

## share-, drive-: sharing and Drive metadata

share-err-single-item = Select a single file or folder
share-err-outside-mount = Not inside the rclone mount ({ $mount })
share-err-mount-itself = Select an item inside the mount, not the mount itself
share-err-not-mounted = The remote is not mounted
share-err-unsupported-remote = Sharing is not supported for this kind of remote
share-err-no-id = The remote did not report an ID for this item

drive-role-owner = Owner
drive-role-editor = Editor
drive-role-commenter = Commenter
drive-role-viewer = Viewer
drive-kind-user = User
drive-kind-group = Group
drive-kind-domain = Domain
drive-kind-anyone = Anyone with link
drive-anyone-with-link = Anyone with the link
drive-err-no-token = rclone has no token for this remote
drive-err-token-unreadable = rclone's token is unreadable
drive-err-denied = Google Drive denied access to this item
drive-err-not-found = Google Drive can't find this item

## cli-: output of the shared `rclone-shell-ext` command line

cli-usage =
    usage: rclone-shell-ext MODE
      --status [--rc-addr HOST:PORT]
      --share-url PATH | --share-open PATH
      --pin PATH | --unpin PATH | --pins
      --offline-repair FOLDER | --offline-remove
      --offline-keeper | --pin-mount ID | --pin-unmount ID   (run by the systemd units)
      --migrate [--dry-run]    move an old cosmic-rclone-ui / rclone-plasma install over
cli-usage-plasma =
    usage: rclone-shell-ext-plasma MODE
      --daemon                 run the D-Bus service
      --status --json [--rc-addr HOST:PORT]
      --menu PATH...           details window (also --details)
      --settings               settings window
    plus the shared modes of `rclone-shell-ext`:
cli-pin-keeping = Keeping { $folder } available offline
cli-pin-stopped = No longer keeping { $folder } offline
cli-repair-requested = Repair of { $folder } requested
cli-offline-still-pinned = { $count } folder(s) are still kept offline; remove them in Settings first
cli-offline-removed = Offline folder services removed
cli-pins-header-on = offline folders: on
cli-pins-header-off = offline folders: off
cli-pin-line = { $folder }: { $phase }
cli-pin-line-error = { $folder }: { $phase } ({ $error })
cli-pin-waiting = { $folder }: waiting for the keeper
cli-status-health = health: { $health }
cli-status-recent = recent: { $count } entries

## migrate-: moving an old cosmic-rclone-ui / rclone-plasma install to the shared identity

migrate-already-done = Already migrated: nothing to do.
migrate-plan-header = Migrating the old { $origin } install to rclone-shell-ext:
migrate-plan-header-none = No old install found; only the marker file is written:
migrate-plan-note = note: { $note }
migrate-step-systemctl = systemctl --user { $args }
migrate-step-systemctl-best-effort = systemctl --user { $args }  (ignored if it fails)
migrate-step-rename = rename { $from } -> { $to }
migrate-step-write = write { $path }
migrate-step-remove = remove { $path }
migrate-step-remove-dir = remove { $path } if empty
migrate-note-other-left = an old { $origin } install was also found; it is left untouched
migrate-note-config-kept = a new config.toml already exists; it is kept as is
migrate-note-old-config-left = the old config file is left in place (delete it when you are happy)
migrate-error-ambiguous = both the old COSMIC and the old Plasma installs have offline folders set up, and it isn't clear which one is in use; turn offline folders off in the one you don't use, then try again
migrate-error-conflict = { $path } already exists and isn't empty, so the old data can't be moved there; move it aside or remove it, then try again. Nothing was changed.
migrate-error-unit-not-ours = { $path } isn't one this app wrote, so it won't be replaced; remove it yourself if it is no longer needed. Nothing was changed.
migrate-error-no-exe = can't find the `rclone-shell-ext` program (next to this one or on PATH), which the new services must run. Nothing was changed.
migrate-error-name-too-long = bisync's working file { $name } would be too long for the file system under the new name. Nothing was changed.
migrate-error-listing-exists = bisync's working files already include { $name }. Nothing was changed.
migrate-error-step-failed = step { $number } of { $total } ({ $step }) failed: { $error }. The old install may be partly moved; run `rclone-shell-ext --migrate` again to finish.
migrate-error-setting = { $path }: not a valid setting ({ $error })
migrate-log-ignored = ignored: { $error }
migrate-log-skipped = skipped { $path }: not there
migrate-line = rclone-shell-ext: { $text }
migrate-done-from = migrated the old { $origin } install.
migrate-done-nothing = migrated (nothing to move).
migrate-failed = rclone-shell-ext: migrating the old install failed: { $error }

## ---- Wave 3 work zones: each agent adds ids ONLY under its own zone (any prefix), so
## ---- parallel branches merge cleanly. The zones are folded into the sections above afterwards.

## zone: cosmic-details

## end zone: cosmic-details

## zone: cosmic-settings

## end zone: cosmic-settings

## zone: panel (health-, panel-, dolphin-, plasmoid)

health-checking = Checking…
health-healthy = Healthy
health-uploading = Uploading
health-downloading = Downloading
health-transferring = Uploading and downloading
health-file-errors = Some files failed
health-cache-full = Cache is full
health-unreachable = Can't reach rclone
health-not-mounted = Not mounted
health-service-stopped = Service stopped
health-uploading-files = Uploading: { $count ->
    [one] 1 file
   *[other] { $count } files
}
health-downloading-files = Downloading: { $count ->
    [one] 1 file
   *[other] { $count } files
}
health-transferring-files = Uploading and downloading: { $count ->
    [one] 1 file
   *[other] { $count } files
}
health-file-errors-files = Some files failed: { $count ->
    [one] 1 file
   *[other] { $count } files
}

panel-title = rclone UI
panel-service-missing = rclone UI service not running
panel-open-folder = Open folder
panel-restart = Restart
panel-start = Start
panel-stop = Stop
panel-settings = Settings
panel-recent-heading = Recently synced
panel-show-all = Show all ({ $count })
panel-show-less = Show less
panel-history-title = Sync history
panel-history-summary = { $count ->
    [one] Last file since rclone started · newest first
   *[other] Last { $count } files since rclone started · newest first
}
panel-history-unavailable = History unavailable
panel-history-empty = Nothing synced since rclone started
panel-last-error = Last error: { $error }
panel-failed = Failed: { $error }
panel-out-of-space = The cache disk is out of space. Free some space so uploads can finish.
panel-outdated-rclone = rclone { $version } is older than the { $minimum } this app needs. Update rclone, then restart the mount; some features may misbehave until you do.

panel-detail-state = Mount: { $mounted ->
    [yes] up
   *[no] down
}   Service: { $running ->
    [yes] running
   *[no] stopped
}
panel-detail-rc-unreachable = rc unreachable: { $error }
panel-detail-rclone-version = rclone { $version }
panel-detail-uploads = Uploads: { $active } in progress, { $queued } queued
panel-detail-cache = Cache: { $files } files, { $size }
panel-detail-errored = Errored files: { $count }
panel-detail-errored-full = Errored files: { $count } (cache out of space)

panel-transfer-line = { $name } — { $done } / { $total } at { $speed }
panel-transfer-line-unknown-size = { $name } — { $done } / ? at { $speed }
panel-speed = { $size }/s
panel-more-transfers = …and { $count } more

panel-caption-join = { $head } · { $tail }
panel-times = ×{ $count }

panel-kind-created = New file
panel-kind-updated = Changed
panel-kind-deleted = Deleted
panel-tip-uploaded = Uploaded to the cloud
panel-tip-downloaded = Downloaded from the cloud
panel-tip-deleted-upload = Deleted here, so removed from the cloud
panel-tip-deleted-download = Deleted in the cloud, so removed here

panel-offline-heading = Offline folders
panel-offline-setup = Offline folders: finish setup in Settings
panel-offline-summary = Offline: { $summary }
panel-pin-waiting = Waiting for the keeper

plasma-config-general = General
plasma-config-retry = Retry
plasma-config-read-failed = Could not read the configuration.
plasma-config-rc-addr = rclone RC address:
plasma-config-env-file = Environment file:
plasma-config-service = systemd service:
plasma-config-mount-path = Mount path:
plasma-config-cache-dir = Cache directory:
plasma-config-sync-interval = Sync interval (seconds):
plasma-config-popup = Popup:
plasma-config-show-all = Show the full history list when opened

dolphin-menu = rclone
dolphin-details = Details…
dolphin-stop-keeping-offline = Stop keeping offline

## end zone: panel

## zone: kirigami (plasma/ui windows)

## end zone: kirigami

## zone: core-errors

## end zone: core-errors
