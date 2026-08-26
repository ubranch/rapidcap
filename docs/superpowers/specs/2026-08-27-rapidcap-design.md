# RapidCap MVP Design

Date: 2026-08-27  
Status: awaiting implementation approval

## Goal

Build a Windows 11 x64 GPUI application that reproduces the user's four daily ShareX workflows with lower startup and interaction overhead:

1. Region screenshot
2. Active-window screenshot
3. Region MP4 recording with system audio
4. Region GIF recording

Parity means matching the approved workflow, hotkeys, capture defaults, output naming, clipboard behavior, and tray behavior. It does not mean reproducing ShareX's UI, editor, upload system, or internal architecture.

## Non-goals

- Uploading, sharing, OCR, annotations, image history, scrolling capture, fullscreen capture, cloud services, telemetry, auto-update, or a settings UI
- Microphone capture in MVP
- Windows 10, macOS, or Linux support
- Pixel-identical ShareX window capture where Windows Graphics Capture produces different borders or shadows
- Runtime encoder selection beyond the approved NVIDIA path

## Product identity and distribution

- Product name: `RapidCap`
- Package slug: `rapidcap`
- Application identifier: `com.inspire.rapidcap`
- Platform: Windows 11 x64
- Distribution: portable local bundle containing `RapidCap.exe`, `ffmpeg.exe`, and required notices/licenses
- Updates: manual replacement only
- Network access: none
- Signing: unsigned local MVP; code signing is required before broad public distribution

FFmpeg redistribution must include the matching license, build information, and source-compliance material required by the selected FFmpeg build. Public release is blocked until that bundle is audited.

## Approved workflow defaults

| Action | Hotkey | Result |
|---|---|---|
| Region screenshot | `Alt+Q` | Select region, save image, write clipboard payload |
| Active-window screenshot | `Alt+Print Screen` | Capture foreground window, save image, write clipboard payload |
| Toggle region video | `Alt+E` and `Shift+Print Screen` | Select/count down/start; same hotkey stops |
| Toggle region GIF | `Ctrl+Shift+Print Screen` | Select/count down/start; same hotkey stops |

Screenshot defaults:

- Encode PNG first
- If PNG exceeds 2048 KB, encode JPEG quality 90 instead
- Hide cursor
- File name: `%pn_%ra{10}`
- Folder: `%USERPROFILE%\Documents\RapidCap\Screenshots\%y-%mo`
- Clipboard: one atomic write containing image data, saved-file drop, and Unicode file path

Video defaults:

- 60 fps
- H.264 NVENC, preset `p7`, tune `hq`, 3000 kbps
- WASAPI system-output loopback, stereo AAC 128 kbps
- 5-second countdown
- Hide cursor
- MP4 fast-start finalization

GIF defaults:

- 15 fps
- Palette generation with `stats_mode=full`
- Dithering `sierra2_4a`
- Hide cursor
- 5-second countdown

## Architecture

Use a two-crate Rust workspace:

- `crates/desktop`: process entrypoint, GPUI windows/tray, commands, application state, hotkeys, lifecycle, and user-facing errors
- `crates/capture`: Windows capture/audio integration, image encoding, FFmpeg supervision, naming, settings, clipboard payload creation, and focused tests

No abstraction layer is added for hypothetical platforms or alternative capture backends. The Windows implementation is the product.

### Runtime flow

```text
Global hotkey or GPUI action
  -> controller validates current state
  -> GPUI region overlay or foreground-window bounds
  -> Windows Graphics Capture (D3D11)
     -> screenshot encoder -> atomic file save -> clipboard STA worker
     -> bounded frame queue + WASAPI loopback -> bundled FFmpeg -> MP4/GIF
```

The GPUI foreground executor owns UI state only. Windows capture callbacks, image encoding, file I/O, WASAPI reads, FFmpeg pipes, and process waits run off the GPUI thread. Background work reports typed events through retained GPUI tasks and weak entity handles.

### State model

```text
Idle -> Selecting -> Countdown -> Recording -> Finalizing -> Idle
  |         |            |           |             |
  +---------+------------+-----------+-------------+-> Error -> Idle
```

Only one capture operation may be active. Screenshot capture skips `Countdown`, `Recording`, and `Finalizing`. `Escape` cancels selection or countdown. Recording hotkeys stop only the matching active recording. New recording commands are rejected while finalizing.

Typed commands:

- `CaptureRegion`
- `CaptureActiveWindow`
- `ToggleVideo`
- `ToggleGif`
- `Cancel`
- `OpenOutputFolder`
- `ShowMainWindow`
- `Exit`

### Capture and encoding

- Use Windows Graphics Capture with a D3D11 device for monitor/window frames.
- Use a virtual-desktop overlay per monitor so mixed-DPI coordinates remain explicit.
- Convert selected logical bounds to physical pixels before cropping.
- Capture the active window identified at command dispatch, not RapidCap's own window.
- Use WASAPI loopback on the current default render endpoint.
- Spawn bundled FFmpeg with redirected stdin/pipes and place it in a Windows Job Object.
- Use bounded queues. Drop video frames rather than blocking capture callbacks; never drop control or finalization events.
- Warm WGC, WASAPI, and FFmpeg during the countdown.
- On stop, close inputs, wait up to 10 seconds, then terminate the Job Object if FFmpeg remains hung.
- Preserve a recoverable temporary output when final encoding or remuxing fails.

MP4 records directly with NVENC/AAC to a temporary file, then performs a fast-start remux and atomic rename. GIF uses the approved palette-generation and palette-use filters. Encoder failures cannot create a zero-byte final file.

### File and clipboard safety

- Create the dated output directory on demand.
- Resolve `%pn` to the captured process name; use `Screen` when no process exists.
- Generate the 10-character random suffix from Windows cryptographic randomness.
- Write to a sibling temporary file, flush/close, then atomically rename to the final name.
- Clipboard writes execute on one dedicated STA thread and set all approved formats during one opened clipboard transaction.
- Clipboard contention retries are short, bounded, and reported if exhausted.
- Capture files remain valid even when clipboard delivery fails.

## GPUI interface

### Main window

- Initial size: 360 x 240 logical pixels; minimum 320 x 220
- Opaque native-looking dark surface; system light/dark preference respected
- Four primary buttons: Region, Window, Video, GIF
- Current status text
- Output-folder button
- Standard close button hides the window to the tray
- `--silent` launches directly to tray
- Tray menu: Show RapidCap, Open Output Folder, Exit

Every interactive element has a stable GPUI element ID, accessible role/name/state, visible keyboard focus, and activation through `Enter`/`Space`. `Tab` follows visual order. High-contrast mode remains usable; reduced-motion preference removes countdown animation.

### Region overlay

- One borderless topmost overlay per monitor
- 20% dimming outside the active selection
- Crosshair, live `x/y/w/h`, and magnifier near the pointer
- Drag-to-select with window/control edge snapping
- `Escape` cancels without output
- Screenshot executes immediately after selection
- Video/GIF enters the 5-second countdown after selection

No overlay effects may obscure the captured pixels. The overlay closes before capture begins.

## Lifecycle and failure behavior

- A named mutex enforces one running instance; a second invocation signals the first instance to show its window, then exits.
- Register global hotkeys at startup. If a hotkey is occupied, show the exact failed combination; main-window buttons remain usable.
- Closing the window hides it; only tray Exit or an explicit exit command terminates RapidCap.
- Unhandled background failures transition to `Error`, display a short actionable message, log technical context, then return to `Idle` after acknowledgement.
- Disk-full, endpoint-loss, and FFmpeg failures stop safely and preserve the best recoverable output.
- Application exit during recording requests finalization, waits up to 10 seconds, then preserves temp output and closes the Job Object.

## Settings, logs, and local data

- Settings: `%APPDATA%\RapidCap\settings.json`
- Logs: `%LOCALAPPDATA%\RapidCap\Logs\rapidcap.log`
- Temporary recordings: `%LOCALAPPDATA%\RapidCap\Temp`
- Captures: `%USERPROFILE%\Documents\RapidCap\Screenshots\%y-%mo`

`settings.json` schema version 1 stores the approved defaults and hotkeys. RapidCap has no settings UI in MVP; advanced edits are manual. Writes are atomic. Missing settings create defaults. Invalid settings fail visibly, preserve the original file, and offer to reopen with safe defaults for the current run. Logs rotate by size and never contain clipboard image data or capture pixels. Temporary files older than seven days are removed at startup only from RapidCap's exact temp directory.

## Toolchain and dependency policy

- Pin Rust in `rust-toolchain.toml`.
- Pin GPUI to one inspected upstream Git revision and commit `Cargo.lock`.
- Record why each Windows API crate and FFmpeg build exists.
- Start from current official GPUI patterns after inspecting the target revision; do not copy examples from a mismatched revision.
- Keep capture code Windows-native and dependency-light. Prefer Win32/WinRT and Rust standard library over wrapper layers where practical.
- Redirect Cargo build artifacts to the workspace `work` directory so final deliverables remain clean.

## Performance targets

Measured on the current Windows 11 + RTX 5080 machine after a warm launch:

- Region overlay visible within 50 ms of hotkey dispatch
- 1080p screenshot saved and clipboard transaction started within 150 ms after selection
- Less than 1% dropped frames during a 60-second 1080p60 NVENC recording
- GPUI event loop remains responsive during encoding and finalization

These are acceptance targets, not guarantees for unrelated hardware.

## Verification

### Automated

- Unit tests: state transitions, hotkey routing, filename expansion, random suffix shape, settings validation/migration, temp/final file rules
- Synthetic encoder test: generated frames plus generated stereo audio produce a readable H.264/AAC MP4 using bundled FFmpeg/NVENC
- Formatting, workspace check, tests, and Clippy with warnings denied for project code

### Windows runtime

- Launch normally and with `--silent`
- Verify single-instance activation and tray lifecycle
- Verify all five registered hotkey combinations, including both video aliases
- Verify region overlay across 100%, 150%, and mixed-DPI monitors
- Verify active-window selection excludes RapidCap
- Verify screenshot clipboard exposes image, file drop, and Unicode path together
- Verify system audio is present and microphone is absent
- Verify stopping and exiting finalize or preserve output within 10 seconds
- Verify clear behavior when NVENC, audio endpoint, output disk, clipboard, or hotkey registration is unavailable

### Visual and accessibility

- Compare main window and overlay screenshots at 100% and 150% scale
- Keyboard-only walkthrough for every action
- Inspect accessible names/roles/states and visible focus
- Check high-contrast and reduced-motion behavior

## Acceptance criteria

MVP is complete when a clean Windows 11 x64 machine can unpack the portable bundle and execute all four workflows without installing ShareX or FFmpeg; outputs match the approved configuration; no capture/encoding work blocks GPUI; failure paths preserve user data; and the runtime, accessibility, and performance checks above pass or have explicit measured exceptions.

