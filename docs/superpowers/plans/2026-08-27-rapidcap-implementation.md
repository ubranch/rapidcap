# RapidCap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a portable Windows 11 x64 GPUI app reproducing the approved ShareX region/window screenshot and region MP4/GIF workflows.

**Architecture:** `rapidcap-desktop` owns GPUI, lifecycle, tray, hotkeys, and overlays. `rapidcap-capture` owns state, settings, naming, WGC frames, clipboard, WASAPI loopback, FFmpeg supervision, and output safety. Capture/encoding work stays off GPUI thread; typed events return through one controller channel.

**Tech Stack:** Rust 1.97.1, GPUI/Zed revision `7733b9922665f103abda7c6a3fde6b9dfdc8eba9`, `windows-capture` 1.5.0, `windows` 0.61.3, CPAL WASAPI, bundled FFmpeg, serde, image, tracing.

## Global Constraints

- Windows 11 x64 only; package slug `rapidcap`; app ID `com.inspire.rapidcap`.
- Portable bundle: `RapidCap.exe`, `ffmpeg.exe`, FFmpeg license/build/source-compliance files.
- No network, telemetry, updater, upload, OCR, editor, history, microphone, or settings UI.
- Hotkeys: `Alt+Q`, `Alt+Print Screen`, `Alt+E`, `Shift+Print Screen`, `Ctrl+Shift+Print Screen`.
- Screenshots: PNG; JPEG quality 90 only when PNG exceeds 2048 KB; cursor hidden.
- MP4: 60 fps, H.264 NVENC `p7`/`hq`, 3000 kbps; WASAPI stereo AAC 128 kbps; 5-second countdown.
- GIF: 15 fps, `stats_mode=full`, `sierra2_4a`; 5-second countdown.
- Name `%pn_%ra{10}` under `%USERPROFILE%\Documents\RapidCap\Screenshots\%y-%mo`.
- Clipboard transaction contains bitmap, file drop, and Unicode path together.
- One active operation; 10-second finalization limit; preserve recoverable temp output.
- No blocking capture, encoding, filesystem, process, or sleep work on GPUI thread.
- Commit `Cargo.lock`; use conventional commits; no history rewrite.

---

## File map

```text
Cargo.toml                         workspace/dependency pins
rust-toolchain.toml               Rust 1.97.1
.cargo/config.toml                target-dir outside deliverable tree
.gitignore                        build/temp exclusions
crates/capture/src/lib.rs         public capture API
crates/capture/src/state.rs       operation state machine
crates/capture/src/settings.rs    schema/defaults/paths/atomic JSON
crates/capture/src/naming.rs      ShareX-compatible output expansion
crates/capture/src/image_file.rs  PNG/JPEG selection + atomic save
crates/capture/src/wgc.rs         WGC frame acquisition/cropping
crates/capture/src/clipboard.rs   one STA clipboard transaction
crates/capture/src/audio.rs       CPAL/WASAPI loopback normalization
crates/capture/src/ffmpeg.rs      named pipes/process/job/finalization
crates/capture/src/recording.rs   video/GIF pipelines
crates/desktop/src/main.rs        startup/single instance/logging
crates/desktop/src/controller.rs  commands/events/GPUI task ownership
crates/desktop/src/window.rs      360x240 accessible main window
crates/desktop/src/overlay.rs     per-monitor region selection/countdown
crates/desktop/src/platform.rs    tray/global hotkeys/foreground HWND
crates/desktop/tests/launch.rs    process smoke checks
scripts/package.ps1               deterministic portable bundle
scripts/verify.ps1                widening validation rings
assets/ffmpeg/                    user-supplied audited FFmpeg payload
```

---

### Task 1: Reproducible workspace and pure domain core

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.cargo/config.toml`
- Create: `.gitignore`
- Create: `crates/capture/Cargo.toml`
- Create: `crates/capture/src/lib.rs`
- Create: `crates/capture/src/state.rs`
- Create: `crates/capture/src/settings.rs`
- Create: `crates/capture/src/naming.rs`
- Create: `crates/desktop/Cargo.toml`
- Create: `crates/desktop/src/main.rs`

**Interfaces:**
- Produces: `CaptureCommand`, `CaptureKind`, `CaptureState`, `CaptureEvent`, `Settings`, `AppPaths`, `OutputNamer`.
- Consumes: no project code.

- [ ] **Step 1: Pin workspace and toolchain**

Create workspace metadata with exact GPUI authority and minimal shared dependencies:

```toml
[workspace]
resolver = "2"
members = ["crates/capture", "crates/desktop"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.97"
license = "MIT"
publish = false

[workspace.dependencies]
anyhow = "1.0.104"
crossbeam-channel = "0.5.15"
cpal = "0.16.0"
gpui = { git = "https://github.com/zed-industries/zed", rev = "7733b9922665f103abda7c6a3fde6b9dfdc8eba9", default-features = false }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "7733b9922665f103abda7c6a3fde6b9dfdc8eba9" }
image = { version = "0.25.10", default-features = false, features = ["jpeg", "png"] }
rand = "0.9.5"
raw-window-handle = "0.6.2"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
tracing = "0.1.44"
tracing-appender = "0.2.4"
tracing-subscriber = { version = "0.3.23", features = ["fmt"] }
windows-capture = "1.5.0"
windows = { version = "0.61.3", features = [
  "Foundation",
  "Graphics_Capture",
  "Graphics_DirectX_Direct3D11",
  "Win32_Foundation",
  "Win32_Graphics_Dwm",
  "Win32_Graphics_Gdi",
  "Win32_Media_Audio",
  "Win32_Security",
  "Win32_Storage_FileSystem",
  "Win32_System_Com",
  "Win32_System_JobObjects",
  "Win32_System_LibraryLoader",
  "Win32_System_Pipes",
  "Win32_System_Threading",
  "Win32_UI_Accessibility",
  "Win32_UI_Shell",
  "Win32_UI_WindowsAndMessaging",
] }
```

Create `rust-toolchain.toml` with `channel = "1.97.1"`, `profile = "minimal"`, and components `rustfmt`, `clippy`. Set `.cargo/config.toml` target directory to `../../../work/rapidcap-target`. Ignore `/dist`, `/assets/ffmpeg/bin`, `*.tmp`, and `*.part`.

- [ ] **Step 2: Write failing state/settings/naming tests**

Add tests asserting:

```rust
assert_eq!(CaptureState::Idle.start(CaptureKind::Video), Ok(CaptureState::Selecting(CaptureKind::Video)));
assert!(CaptureState::Finalizing(CaptureKind::Video).start(CaptureKind::Gif).is_err());
assert_eq!(Settings::default().video.fps, 60);
assert_eq!(Settings::default().gif.fps, 15);
assert_eq!(OutputNamer::for_test("0000000000").file_stem("Code"), "Code_0000000000");
```

Also assert default screenshot threshold `2_097_152`, JPEG quality `90`, video bitrate `3_000_000`, audio bitrate `128_000`, countdown `5s`, and both video aliases.

- [ ] **Step 3: Run tests and confirm red**

Run: `cargo +1.97.1 test -p rapidcap-capture --lib`

Expected: compile failure because domain types do not exist.

- [ ] **Step 4: Implement minimal pure core**

Define:

```rust
pub enum CaptureCommand { CaptureRegion, CaptureActiveWindow, ToggleVideo, ToggleGif, Cancel }
pub enum CaptureKind { RegionScreenshot, ActiveWindowScreenshot, Video, Gif }
pub enum CaptureState { Idle, Selecting(CaptureKind), Countdown(CaptureKind, u8), Recording(CaptureKind), Finalizing(CaptureKind), Error(String) }
pub enum CaptureEvent { StateChanged(CaptureState), OutputSaved(PathBuf), ClipboardFailed(String), Failed(String) }
```

`CaptureState::start` accepts only `Idle`; `cancel` accepts `Selecting`/`Countdown`; `stop` accepts matching `Recording`. Implement `Settings::default`, schema version validation, `AppPaths::discover` with Windows known folders, month folder `%y-%mo`, process-name sanitization, and OS-random alphanumeric suffix generation. `for_test(&str)` injects one validated 10-character suffix.

- [ ] **Step 5: Verify and commit**

Run:

```powershell
cargo +1.97.1 fmt --all --check
cargo +1.97.1 test -p rapidcap-capture --lib
cargo +1.97.1 check --workspace
git add Cargo.toml Cargo.lock rust-toolchain.toml .cargo .gitignore crates
git commit -m "feat: establish RapidCap workspace and capture domain"
```

Expected: all commands exit `0`.

---

### Task 2: Settings persistence, logging, and output safety

**Files:**
- Modify: `crates/capture/src/settings.rs`
- Modify: `crates/capture/src/lib.rs`
- Create: `crates/capture/src/image_file.rs`
- Modify: `crates/desktop/src/main.rs`

**Interfaces:**
- Consumes: `Settings`, `AppPaths`, `OutputNamer`.
- Produces: `SettingsStore::load(&self) -> Result<Settings, SettingsError>`, `SettingsStore::save(&self, &Settings) -> Result<(), SettingsError>`, `save_screenshot(...) -> Result<PathBuf>`, rotating file logger.

- [ ] **Step 1: Write failing persistence and encoder tests**

Use a unique test directory. Assert missing config returns and writes defaults; corrupt JSON remains unchanged and returns `SettingsError::Invalid`; save replaces atomically; 16x16 solid image stays PNG; seeded noisy image above threshold becomes JPEG; failed write leaves no final file.

```rust
let saved = save_screenshot(&rgba, 16, 16, &base, 2_097_152, 90)?;
assert_eq!(saved.extension().unwrap(), "png");
assert!(!base.with_extension("part").exists());
```

- [ ] **Step 2: Run focused tests and confirm red**

Run:

```powershell
cargo +1.97.1 test -p rapidcap-capture settings
cargo +1.97.1 test -p rapidcap-capture image_file
```

Expected: unresolved `SettingsStore` and `save_screenshot`.

- [ ] **Step 3: Implement atomic settings and screenshot writes**

`SettingsStore::save` serializes pretty JSON to `settings.json.part`, calls `sync_all`, then `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`. `load` never overwrites invalid JSON. `save_screenshot` encodes PNG into memory first; if bytes exceed threshold, re-encodes RGB JPEG quality 90; writes `.part`, flushes, closes, and atomically renames. Any error removes only newly created `.part`.

- [ ] **Step 4: Add bounded diagnostics**

Initialize non-blocking tracing before GPUI startup. Write `%LOCALAPPDATA%\RapidCap\Logs\rapidcap.log`; roll daily; retain seven files during startup by enumerating only exact log directory. Log state transitions, hotkey registration failure, capture/encoder errors, output path, and elapsed timings. Never log pixels, clipboard data, or environment contents.

- [ ] **Step 5: Verify and commit**

Run `cargo +1.97.1 test -p rapidcap-capture`, `cargo +1.97.1 check --workspace`, then commit:

```powershell
git add crates/capture crates/desktop Cargo.lock
git commit -m "feat: persist settings and save screenshots safely"
```

---

### Task 3: GPUI main window, controller, and accessibility

**Files:**
- Create: `crates/desktop/src/controller.rs`
- Create: `crates/desktop/src/window.rs`
- Modify: `crates/desktop/src/main.rs`
- Modify: `crates/desktop/Cargo.toml`

**Interfaces:**
- Consumes: `CaptureCommand`, `CaptureState`, `CaptureEvent`, `SettingsStore`.
- Produces: `AppController::dispatch`, `CommandError::{Busy, InvalidState, WorkerUnavailable}`, `MainWindow`, retained GPUI `Task`s.

- [ ] **Step 1: Write failing controller and GPUI tests**

Pure test: dispatching `CaptureRegion` from `Idle` emits `Selecting(RegionScreenshot)`; dispatch during `Finalizing` returns `Busy`. GPUI test opens `MainWindow`, asserts stable IDs `capture-region`, `capture-window`, `record-video`, `record-gif`, `open-output`, then dispatches `CaptureRegion` and observes controller event.

- [ ] **Step 2: Run tests and confirm red**

Run: `cargo +1.97.1 test -p rapidcap-desktop`

Expected: missing `controller` and `window` modules.

- [ ] **Step 3: Implement controller ownership**

`AppController` owns `CaptureState`, `Settings`, output paths, worker sender/receiver, generation counter, and retained `Vec<Task<()>>`. `dispatch` updates state synchronously, calls `cx.notify()`, then uses `cx.background_spawn` for blocking work. Completion uses `WeakEntity<AppController>` and ignores stale generations.

```rust
pub fn dispatch(&mut self, command: CaptureCommand, cx: &mut Context<Self>) -> Result<(), CommandError>;
pub fn apply_event(&mut self, event: CaptureEvent, cx: &mut Context<Self>);
```

- [ ] **Step 4: Implement minimal accessible window**

Open 360x240, minimum 320x220, opaque background, native titlebar, app ID `com.inspire.rapidcap`. Render four buttons, status, output folder. Each button has stable element/accessibility ID, `Role::Button`, label, tab stop, focus ring, click handler, and Enter/Space action. Register `on_window_should_close` returning `false` after hiding window. Use system appearance tokens only; no component framework or animation system.

- [ ] **Step 5: Verify launch and commit**

Run tests/checks, then `cargo +1.97.1 run -p rapidcap-desktop`. Confirm window opens and buttons change status without blocking. Commit:

```powershell
git add crates/desktop Cargo.lock
git commit -m "feat: add accessible GPUI capture controller"
```

---

### Task 4: Windows lifecycle, tray, and global hotkeys

**Files:**
- Create: `crates/desktop/src/platform.rs`
- Modify: `crates/desktop/src/controller.rs`
- Modify: `crates/desktop/src/main.rs`
- Create: `crates/desktop/tests/launch.rs`

**Interfaces:**
- Consumes: `AppController::dispatch`, GPUI `Window: HasWindowHandle`.
- Produces: `PlatformRuntime`, `HotkeyId`, single-instance activation, tray commands.

- [ ] **Step 1: Write failing mapping and process tests**

Assert exact IDs/modifiers/keys:

```rust
assert_eq!(HotkeyId::Region.registration(), (MOD_ALT, 0x51));
assert_eq!(HotkeyId::Window.registration(), (MOD_ALT, VK_SNAPSHOT.0 as u32));
assert_eq!(HotkeyId::VideoAltE.registration(), (MOD_ALT, 0x45));
assert_eq!(HotkeyId::VideoPrint.registration(), (MOD_SHIFT, VK_SNAPSHOT.0 as u32));
assert_eq!(HotkeyId::Gif.registration(), (MOD_CONTROL | MOD_SHIFT, VK_SNAPSHOT.0 as u32));
```

Launch test starts `RapidCap.exe --probe`, expects JSON containing app ID/version/config/output paths, exit `0`, and no second process.

- [ ] **Step 2: Run tests and confirm red**

Run:

```powershell
cargo +1.97.1 test -p rapidcap-desktop platform
cargo +1.97.1 test -p rapidcap-desktop --test launch
```

Expected: missing `PlatformRuntime`/`--probe`.

- [ ] **Step 3: Implement one Win32 message runtime**

Create one dedicated STA thread with hidden message-only HWND. It owns named mutex `Local\com.inspire.rapidcap`, registered hotkeys, tray `NOTIFYICONDATAW`, and single-instance named pipe. Convert `WM_HOTKEY` and tray menu IDs into `CaptureCommand`/lifecycle messages through `crossbeam_channel`. If mutex already exists, connect to activation pipe, send `ShowMainWindow`, exit `0`. Unregister every successful hotkey and remove tray icon on shutdown.

- [ ] **Step 4: Wire GPUI lifecycle**

Poll platform messages through one retained async task. `ShowMainWindow` calls `cx.activate(true)` and activates stored GPUI window. `OpenOutputFolder` uses `ShellExecuteW`. `Exit` asks controller to finalize; quits only after `Idle` or 10-second deadline. `--silent` opens main window with `show: false`. Registration errors list exact hotkeys; buttons remain active.

- [ ] **Step 5: Verify and commit**

Run tests. Launch `--silent`, verify tray icon, invoke second process, verify first window appears. Confirm occupied-hotkey error by temporarily registering one test combination inside launch test. Commit:

```powershell
git add crates/desktop
git commit -m "feat: add Windows tray and global hotkeys"
```

---

### Task 5: Region overlay and capture target resolution

**Files:**
- Create: `crates/desktop/src/overlay.rs`
- Modify: `crates/desktop/src/controller.rs`
- Modify: `crates/desktop/src/main.rs`
- Create: `crates/capture/src/wgc.rs`
- Modify: `crates/capture/src/lib.rs`

**Interfaces:**
- Consumes: `CaptureKind`, GPUI displays/window bounds, Win32 foreground/window rectangle APIs.
- Produces: `PhysicalRegion`, `CaptureTarget::{Region, Window}`, `select_region`, `foreground_window_target`.

- [ ] **Step 1: Write failing geometry tests**

Test negative virtual-desktop coordinates, 100%/150% scaling, reverse drag, minimum 2x2 selection, and clamping. Test foreground target rejects RapidCap HWND and falls back to previous foreground HWND captured before main window activation.

```rust
assert_eq!(PhysicalRegion::from_drag((-100, 50), (-500, 350)), PhysicalRegion { x: -500, y: 50, width: 400, height: 300 });
```

- [ ] **Step 2: Run focused tests and confirm red**

Run:

```powershell
cargo +1.97.1 test -p rapidcap-desktop overlay
cargo +1.97.1 test -p rapidcap-capture wgc
```

Expected: missing region/target types.

- [ ] **Step 3: Implement per-monitor GPUI overlays**

Open one borderless topmost overlay per `cx.displays()` with exact physical monitor bounds. Render 20% dim outside selection, crosshair, dimensions, and nearest-neighbor magnifier. Pointer down stores origin; move normalizes selection and applies Win32 `WindowFromPoint` + `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` snapping within 8 physical pixels; pointer up emits one `PhysicalRegion`. Escape emits cancel. Close every overlay before dispatching capture/countdown.

- [ ] **Step 4: Implement target resolution**

`foreground_window_target(rapidcap_hwnds)` calls `GetForegroundWindow`, rejects hidden/cloaked/owned RapidCap windows, obtains extended frame bounds and process name, returns HWND plus physical crop. Region target resolves intersecting monitor through `windows_capture::monitor::Monitor`; WGC captures cursor off and border off, then crops CPU BGRA only after copying frame data out of WGC frame lifetime.

- [ ] **Step 5: Verify mixed-DPI behavior and commit**

Run focused tests. Launch on 100% and 150% scale; verify negative-coordinate monitor. Escape must leave no file. Commit:

```powershell
git add crates/desktop/src/overlay.rs crates/desktop/src/controller.rs crates/desktop/src/main.rs crates/capture/src/wgc.rs crates/capture/src/lib.rs
git commit -m "feat: add region selection and capture targets"
```

---

### Task 6: Screenshot workflow and atomic clipboard payload

**Files:**
- Create: `crates/capture/src/clipboard.rs`
- Modify: `crates/capture/src/wgc.rs`
- Modify: `crates/capture/src/image_file.rs`
- Modify: `crates/capture/src/lib.rs`
- Modify: `crates/desktop/src/controller.rs`

**Interfaces:**
- Consumes: `CaptureTarget`, `OutputNamer`, `save_screenshot`.
- Produces: `SavedCapture { path: PathBuf, rgba: Vec<u8>, width: u32, height: u32 }`, `capture_screenshot(target, settings) -> Result<SavedCapture>`, `ClipboardWorker::write(ClipboardRequest) -> Result<()>`.
- Test seams: `FrameSource::capture(&self, CaptureTarget) -> Result<RawFrame>` and `ClipboardSink::write(&self, ClipboardRequest) -> Result<()>`; production implementations are WGC and `ClipboardWorker`.

- [ ] **Step 1: Write failing screenshot and clipboard-layout tests**

Use a synthetic 2x2 BGRA frame to verify channel conversion, crop stride, DIBV5 bottom-up layout, UTF-16 path with NUL, and `DROPFILES` offset. Add an integration test around an injected `FrameSource` and `ClipboardSink` proving save succeeds when clipboard fails and returns `CaptureEvent::ClipboardFailed` after `OutputSaved`.

- [ ] **Step 2: Run focused tests and confirm red**

Run:

```powershell
cargo +1.97.1 test -p rapidcap-capture screenshot
cargo +1.97.1 test -p rapidcap-capture clipboard
```

Expected: missing clipboard payload/worker.

- [ ] **Step 3: Implement single-frame WGC capture**

Start `windows-capture` with `CursorCaptureSettings::WithoutCursor`, `DrawBorderSettings::WithoutBorder`, BGRA/RGBA8, bounded channel capacity 1. First valid frame copies selected region, stops capture control, and closes session. Timeout after 2 seconds returns error and writes nothing. Pass pixels to `save_screenshot` using resolved process name/path.

- [ ] **Step 4: Implement one STA clipboard transaction**

Dedicated clipboard thread receives `ClipboardRequest { rgba, width, height, path }`. Allocate `HGLOBAL` blocks before opening clipboard. Retry `OpenClipboard` 5 times at 10 ms. Once open: `EmptyClipboard`; set `CF_DIBV5`, `CF_HDROP`, and registered `Preferred DropEffect`, plus `CF_UNICODETEXT` path; close once. Ownership transfers only after successful `SetClipboardData`; free non-transferred blocks. Return error without touching saved file.

- [ ] **Step 5: Verify real workflows and commit**

Run capture tests. Launch app; execute `Alt+Q` and `Alt+Print Screen`; paste into Paint, Explorer, and Notepad. Verify same saved path and no transient multi-open sequence. Commit:

```powershell
git add crates/capture crates/desktop/src/controller.rs
git commit -m "feat: capture screenshots and publish clipboard payload"
```

---

### Task 7: FFmpeg video/GIF supervision and WASAPI loopback

**Files:**
- Create: `crates/capture/src/audio.rs`
- Create: `crates/capture/src/ffmpeg.rs`
- Create: `crates/capture/src/recording.rs`
- Modify: `crates/capture/src/lib.rs`
- Modify: `crates/desktop/src/controller.rs`
- Modify: `crates/desktop/src/overlay.rs`

**Interfaces:**
- Consumes: `CaptureTarget`, `CaptureState`, WGC BGRA frames, `Settings`.
- Produces: `RecordingSession::start(RecordingRequest) -> Result<RecordingSession>`, `RecordingSession::stop(self, Duration) -> Result<RecordingOutput>`, `FfmpegCommand { program: PathBuf, args: Vec<OsString> }`, `AudioPacket { samples: Vec<i16>, sample_rate: u32, channels: u16 }`.

- [ ] **Step 1: Write failing command/state/audio tests**

Assert video command includes two named-pipe inputs plus:

```text
-f rawvideo -pixel_format bgra -framerate 60 -c:v h264_nvenc -preset p7 -tune hq -b:v 3000k -c:a aac -b:a 128k
```

Assert GIF command includes `fps=15,palettegen=stats_mode=full` and `paletteuse=dither=sierra2_4a`. Test stereo normalization for `f32`, `i16`, mono duplication, and silence packets. Test state: select -> countdown 5..1 -> recording -> finalizing -> idle; same hotkey stops; mismatched hotkey returns busy.

- [ ] **Step 2: Run tests and confirm red**

Run:

```powershell
cargo +1.97.1 test -p rapidcap-capture recording
cargo +1.97.1 test -p rapidcap-capture ffmpeg
cargo +1.97.1 test -p rapidcap-capture audio
```

Expected: missing recording modules.

- [ ] **Step 3: Implement FFmpeg supervisor**

Create unique `\\.\pipe\RapidCap-{pid}-{nonce}-video` and `-audio` servers with `CreateNamedPipeW`; allow only current-user SID. Spawn exact bundled `ffmpeg.exe` with inherited job assignment, hidden window, stderr pipe, temp output, and `-movflags +faststart`. Connect/write pipes from dedicated threads. Keep last 200 stderr lines. Stop closes producers/pipes, waits 10 seconds, terminates Job Object on timeout, validates output with `ffmpeg -v error -i <file> -f null -`, then atomically renames. On failure preserve `.part` and return its path.

- [ ] **Step 4: Implement WGC cadence and WASAPI loopback**

WGC callback copies cropped BGRA into a capacity-3 channel and returns immediately. Video writer uses QPC timestamps to emit/duplicate frames on a 60 Hz timeline; records dropped/duplicated counters. CPAL opens default output device as WASAPI loopback, prefers 48 kHz stereo, converts samples to interleaved signed 16-bit PCM, and writes capacity-8 audio packets. Endpoint loss stops recording with recoverable temp; no microphone device is opened.

For GIF, omit audio pipe and use one FFmpeg filter-complex pass:

```text
split[a][b];[a]fps=15,palettegen=stats_mode=full[p];[b]fps=15[x];[x][p]paletteuse=dither=sierra2_4a
```

- [ ] **Step 5: Integrate countdown, verify, and commit**

During countdown start WGC device/audio capability probe/FFmpeg process and display `5..1`; begin pipe delivery at zero. Run synthetic NVENC test for 10 seconds and inspect streams with `ffprobe`: H.264 60 fps, AAC stereo 128 kbps. Record 60 seconds 1080p60 with system audio; calculate dropped ratio `<1%`. Create GIF and inspect 15 fps. Commit:

```powershell
git add crates/capture crates/desktop/src/controller.rs crates/desktop/src/overlay.rs
git commit -m "feat: record NVENC video and GIF with system audio"
```

---

### Task 8: Recovery, packaging, and acceptance verification

**Files:**
- Modify: `crates/capture/src/recording.rs`
- Modify: `crates/desktop/src/controller.rs`
- Modify: `crates/desktop/src/window.rs`
- Create: `scripts/package.ps1`
- Create: `scripts/verify.ps1`
- Create: `assets/ffmpeg/README.txt`
- Create: `LICENSE`

**Interfaces:**
- Consumes: all completed runtime interfaces.
- Produces: `dist/RapidCap/` portable artifact and verification report output.

- [ ] **Step 1: Write failing recovery/launch checks**

Tests inject disk-full, clipboard lock, missing FFmpeg, FFmpeg exit 1, hotkey conflict, audio endpoint loss, and stop timeout. Assert: no zero-byte final; `.part` preserved for encoder failures; state returns to `Idle` after acknowledgement; busy commands rejected during finalization; exit deadline ≤10 seconds.

- [ ] **Step 2: Run recovery tests and confirm failures**

Run:

```powershell
cargo +1.97.1 test --workspace recovery
cargo +1.97.1 test -p rapidcap-desktop --test launch
```

Expected: each injected boundary fails until mapped into explicit `CaptureEvent` and cleanup behavior.

- [ ] **Step 3: Complete recovery and accessibility states**

Map every injected failure to short UI status plus detailed log. Add Retry/Open Temp/Open Output actions only where output exists. Ensure focus returns to initiating control after cancel/error; high-contrast uses solid borders/colors; reduced motion renders static countdown digits. Cleanup removes only RapidCap temp files older than seven days after verifying canonical temp directory prefix.

- [ ] **Step 4: Package exact portable bundle**

`scripts/package.ps1` must:

1. Require `assets/ffmpeg/bin/ffmpeg.exe`, `ffprobe.exe`, `LICENSE.txt`, `BUILD.txt`, `SOURCE.txt`; fail if any missing.
2. Run `cargo +1.97.1 build -p rapidcap-desktop --release --locked`.
3. Recreate only repository-local `dist/RapidCap` after resolving path under repository root.
4. Copy binary as `RapidCap.exe`, FFmpeg files, `LICENSE`, and notices.
5. Emit SHA-256 checksums and fail if `RapidCap.exe --probe` exits nonzero.

Do not download FFmpeg or invent compliance files. User supplies/audits payload before package step.

- [ ] **Step 5: Run final rings and commit**

Run:

```powershell
cargo +1.97.1 fmt --all --check
cargo +1.97.1 check --workspace --locked
cargo +1.97.1 test --workspace --locked
cargo +1.97.1 clippy --workspace --all-targets --locked -- -D warnings
pwsh -File scripts/verify.ps1
pwsh -File scripts/package.ps1
git status --short
```

`verify.ps1` checks normal/`--silent` launch, single instance, tray, all five hotkeys, region/window screenshots, clipboard formats, MP4 video+system audio, GIF 15 fps, 100%/150% DPI screenshots, keyboard focus, stop deadline, and measured performance. Record unavailable hardware/manual accessibility checks as explicit failures, never passes. Commit only project files, not `dist` or user-supplied FFmpeg binaries:

```powershell
git add crates scripts assets/ffmpeg/README.txt LICENSE Cargo.lock
git commit -m "build: package and verify RapidCap portable release"
```

## Completion gate

Implementation is complete only when Task 8 checks pass and portable bundle runs on a clean Windows 11 x64 account without ShareX or system FFmpeg. Local compile/tests do not prove runtime capture, clipboard, audio, DPI, NVENC, or portable compliance.
