# RapidCap

A screen capture panel for Windows and macOS. Region and window screenshots to
the clipboard or disk, screen recording to MP4, and GIF capture, from a single
compact window that stays out of the way and a set of global shortcuts that
work whether or not it is focused.

Written in Rust on [GPUI](https://www.gpui.rs/), the UI framework behind Zed.

## Shortcuts

| Action | Windows | macOS |
| --- | --- | --- |
| Capture region | `Alt+E` | `Option+E` |
| Capture active window | `Ctrl+Shift+W` | `Cmd+Shift+W` |
| Start / stop recording | `Alt+Q` | `Option+Q` |
| Start / stop GIF | `Ctrl+Shift+G` | `Cmd+Shift+G` |
| Pause / resume | `Ctrl+Shift+P` | `Cmd+Shift+P` |

`Alt+E` and `Alt+Q` keep the same physical keys on both platforms because they
were chosen as keys. The other three follow the platform's convention for a
three-finger chord, which is Ctrl on Windows and Command on macOS.

Shortcut registration is first-come-first-served on both systems. If ShareX or
another capture tool already holds one of these, RapidCap logs the conflict and
carries on; the panel's own buttons still work.

## Building

Rust 1.97.1, pinned in `rust-toolchain.toml`. FFmpeg has to be on `PATH` for
recording and GIF capture — screenshots do not need it.

```sh
cargo build --release
```

### Windows

```powershell
pwsh -File scripts/package.ps1
```

Vendored FFmpeg binaries live under `assets/ffmpeg`; the build falls back to
`PATH` when they are absent.

### macOS

```sh
scripts/package-macos.sh
```

This produces `dist/RapidCap.app`. The bundle is not cosmetic: macOS records the
Screen Recording permission against a bundle identifier and its code signature,
so a bare binary launched from a terminal makes the *terminal* the grantee and
every capture comes back empty.

The script signs ad-hoc (`codesign --sign -`), which keeps the permission
attached across rebuilds on the machine that built it. Distributing to another
Mac needs a Developer ID and notarisation, which the script deliberately leaves
to whoever holds the certificate:

```sh
codesign --force --deep --options runtime \
  --sign "Developer ID Application: YOUR NAME (TEAMID)" dist/RapidCap.app
ditto -c -k --keepParent dist/RapidCap.app dist/RapidCap.zip
xcrun notarytool submit dist/RapidCap.zip --keychain-profile YOUR_PROFILE --wait
xcrun stapler staple dist/RapidCap.app
```

Homebrew's FFmpeg (`brew install ffmpeg`) is what `PATH` resolution expects.

## First run on macOS

Open the app once and grant Screen Recording in System Settings → Privacy &
Security. Nothing else needs a permission: the shortcuts go through Carbon's
`RegisterEventHotKey`, which is not an Accessibility client, and macOS exposes
no system-audio input device, so recordings are video only.

## Testing

```sh
cargo test --workspace
```

CI runs formatting, Clippy and the test suite on both `windows-latest` and
`macos-14`, and builds the macOS bundle. What CI cannot cover is anything behind
the Screen Recording prompt, since a runner has no one to grant it. Those checks
are manual, on a real login session:

1. Press the capture shortcut and confirm the region overlay appears.
2. Record a few seconds and confirm the file is not blank.
3. Confirm the recording control bar is *absent* from the resulting file.
4. Confirm the tray icon menu opens and quits the app.

## Licence

MIT.
