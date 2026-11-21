# Supported platforms

What aidb runs on, what is tested before each release, and what is known not to work yet.
CI builds and tests every package on every push (see [architecture](architecture.md#build-and-ci)).

## Desktop

| Platform | Packages                                            | Minimum                                              | Tested in CI                                                        |
| -------- | --------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------- |
| Windows  | x64: MSI (per machine), NSIS installer (per user)   | Windows 10 or 11, x64, with Microsoft Edge WebView2  | GitHub's `windows-latest` image                                     |
| macOS    | `.dmg` for Apple Silicon and for Intel, built apart | macOS 13 Ventura                                     | GitHub's `macos-latest` (Apple Silicon) and `macos-15-intel` images |
| Linux    | x64: `.deb`, `.rpm`, AppImage                       | glibc 2.35 and WebKitGTK 4.1 (Ubuntu 22.04 or newer) | Ubuntu 22.04 and 24.04 (AppImage, `.deb`); Fedora 45 (`.rpm`)       |

"Tested" means, for every package: installed with the platform's own installer on a clean
machine, started, killed and started again (the local database must recover), quit, upgraded
to the next version with user data kept, and uninstalled. The Rust tests, the OS credential
store, the PostgreSQL integration tests and the secret scan also run on Windows x64, macOS
Apple Silicon and macOS Intel; Linux runs them against every supported PostgreSQL version.

Notes per platform:

- **Windows**: both installers install WebView2 when it is missing (it ships with Windows 11
  and recent Windows 10 updates). Passwords and API keys go to Windows Credential Manager.
- **macOS**: the app declares macOS 13 as its minimum, since the interface needs the WebKit of
  Safari 16.4 or newer. Passwords and API keys go to the login Keychain.
- **Linux**: packages are built on Ubuntu 22.04, the oldest supported release, so they run on
  it and on newer distributions. The `.deb` and `.rpm` declare WebKitGTK and its dependencies.
  The AppImage carries the app's libraries but, like every AppImage, uses the system's
  OpenGL/EGL drivers and fonts, which desktop distributions install by default. Saving
  passwords needs a Secret Service (GNOME Keyring, or KWallet with its Secret Service
  interface); without one, aidb keeps them for the session only and says so in Settings ›
  Diagnostics. CI runs on X11 (Xvfb); Wayland sessions are expected to work through WebKitGTK
  but are not tested.

## PostgreSQL

| Major | Exact versions tested (November 2026)            |
| ----- | ------------------------------------------------ |
| 15    | 15.20                                            |
| 16    | 16.16                                            |
| 17    | 17.12 (Linux, and natively on Windows and macOS) |
| 18    | 18.7                                             |

Each run uses the newest minor release of the official `postgres` image; the CI log of a run
records the exact server version. Other versions may connect: testing a connection then says
the server is outside the tested range. Only PostgreSQL is supported; other databases are not.

## Not supported yet

- **Windows on Arm, Linux on Arm**: not packaged until they can be built and tested in CI.
- **Phones and tablets**: the interface works at phone widths, but there are no Android or iOS
  builds.
- **Signed releases**: macOS bundles are signed ad hoc and Windows installers are unsigned
  until release certificates are in place, so macOS asks for confirmation the first time
  (Control-click, then Open, or System Settings › Privacy & Security › Open Anyway) and
  Windows SmartScreen warns. See the [release checklist](release-checklist.md#signing).
- **Automatic updates**: none. Install a newer package over the old one; settings, profiles and
  history are kept.
- **Uninstalling** removes the program and keeps the data folder, as each platform's installers
  do. To remove everything, clear it first in Settings › Storage, forget saved passwords and
  keys (Settings › Connections and AI), then delete the folders listed in
  [privacy](privacy.md#where-your-data-is).
- **One open connection at a time**, as designed for the first release.
