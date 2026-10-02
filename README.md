# ship-shape

Auto-updater library for wxDragon desktop apps, supporting Windows, macOS, and Linux.

## Features

- Check for stable (semver) or dev (commit hash) updates from GitHub Releases
- Download with progress callback
- Minisign signature verification before applying
- `ui` module: update/progress dialogs, plus platform-specific install flows:
  - Windows: PowerShell install/extract scripts that relaunch the app afterward
  - macOS: privately mounts the verified `.dmg`, stages and validates its matching app bundle,
    then replaces and relaunches writable installations; protected, mounted-image, or translocated
    installations retain manual DMG instructions
  - Linux: a shell script that extracts the tarball or replaces the running `.AppImage`, then
    relaunches the app, the same self-updating flow as Windows
  - Other platforms: downloads the file and tells the user where it is

On macOS the expected release asset is `{app_name}.dmg` (`install_kind` is ignored there,
since there's only one asset kind). Windows and Linux keep the same `InstallKind` distinction:
`{app_name}.zip` / `{app_name}.tar.gz` for `InstallKind::Portable`, and `{app_name}_setup.exe` /
`{app_name}.AppImage` for `InstallKind::Installer`. The Linux installer flow requires the running
process to be inside an AppImage (it reads the `APPIMAGE` environment variable the AppImage
runtime sets); there's no separate install step to run, so the update just replaces that file.

Each platform's asset names, download folder, and install flow live in one file under
`src/platform/`. To add a platform, add a file there and select it in `src/platform.rs`.

## Usage

```toml
[dependencies]
ship-shape = "0.3.0"
```

```rust
use std::sync::Arc;
use ship_shape::{InstallKind, UpdateChannel, UpdaterConfig, ui::{self, CheckTrigger}};

let config = Arc::new(
    UpdaterConfig::new(
        "owner/repo",
        "myapp",
        "My App",
        "RWQ...minisign-public-key...",
        env!("CARGO_PKG_VERSION"),
    )
    .with_commit(env!("MY_APP_COMMIT_HASH"))
    .with_install_kind(InstallKind::Portable),
);
ui::run_update_check(config, &frame, UpdateChannel::Stable, CheckTrigger::Manual);
```

## Upgrading from 0.2

- `UpdaterConfig::new` takes the current version instead of the user agent. The user agent
  now defaults to `"{app_name}/{version}"`; override it with `with_user_agent`.
- The commit hash and `is_installer` moved into the config: `with_commit` and
  `with_install_kind`.
- `check_for_updates(config, channel)` and
  `ui::run_update_check(config, &frame, channel, trigger)` lost their other arguments.
  `silent: true` is now `CheckTrigger::Automatic`.
- `UpdateError` variants dropped their `Error` suffix (`HttpError` is now `Http(u16)`), gained
  `Io`, and the enum is `#[non_exhaustive]`.

## macOS installation

Updates replace the running app at its existing location, including custom folders and renamed
bundles. The DMG must contain exactly one top-level `.app` matching the running bundle identifier,
with a valid code signature and executable. Mounted images and Gatekeeper-translocated apps are
not modified. No administrator privileges are requested.

Mounting, validation, copying, and the helper handshake run off the UI thread. Canceling before
shutdown drops the staged update. The detached helper waits up to 60 seconds for the app to exit,
then swaps bundles using same-filesystem renames. A failed replacement restores the old bundle;
a failed restore or launch preserves the backup and reports its location. Helper logs survive
cleanup under `~/Library/Logs/ship-shape-update-*.log`. A successful Launch Services request does
not detect subsequent application crashes.

Use `ui::run_update_check_with_exit_handler` to save state and terminate normally. The existing
`run_update_check` entry point keeps its immediate process-exit behavior. The handler runs on the
UI thread only after successful installer preparation. It must terminate the process within
60 seconds; simply hiding the main window is insufficient.

`UpdaterConfig::with_macos_relaunch_env(name, value)` preserves an explicit environment override
when launching through macOS `open`, such as an isolated settings directory. It is ignored on
other platforms; values are passed as arguments and never interpolated into shell code.

For local testing, build `cargo build --example macos_update_demo`. Run the example with
`--setup /tmp/ship-shape-demo` using a new directory, then run the generated
`Installed Demo.app/Contents/MacOS/demo` executable with `--cancel` or `--apply` followed by the
fixture's `demo.dmg` path. Applying must display version 2; canceling must retain version 1.
The demo trusts its own local, ad-hoc signed fixture; it does not change production verification.

The download gauge stays below 100 until verification and preparation finish. Native progress
updates can yield to the event loop, so completion waits for the active Update/Pulse call to
return before destroying the dialog or calling the shutdown handler. Regression tests cover this
ordering without requiring a graphical session.

Run `cargo test` on macOS for coverage of cancellation, helper timeouts, replacement and rollback,
failed restore and launch, competing destination changes, protected locations, bundle selection,
signature tampering, and paths with spaces and apostrophes. Tests substitute launch commands and
alerts and only modify temporary fixture apps. The demo above additionally exercises actual DMG
mounting and Launch Services; testing a signed, notarized release establishes the Gatekeeper path.

## License

MIT
