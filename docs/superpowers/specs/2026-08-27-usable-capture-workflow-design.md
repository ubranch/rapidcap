# Usable Capture Workflow

## Goal

Make Window, Video, and GIF understandable without prior knowledge: user sees target selection, recording start, active target, elapsed time, and stop control.

## Interaction

- Window opens fullscreen selector. Hover highlights detected window and shows process/window name. Click captures highlighted window. Escape cancels.
- Video and GIF open same selector. Click highlighted window or drag region. Selected boundary stays visible.
- Five-second countdown appears inside selected boundary. Recording starts only after countdown reaches zero.
- Active recording shows persistent red boundary plus compact HUD containing capture type, target name or dimensions, elapsed time, Stop, and Cancel.
- Start hotkey becomes stop hotkey while matching recording is active. Stop finalizes output. Cancel terminates recording and removes partial output.
- Completion closes selector/HUD, restores compact main window, reports saved path, and leaves no FFmpeg process.

## Architecture

- Reuse existing GPUI overlay for target selection; add hover-based Win32 window discovery beside existing drag-region selection.
- Add one GPUI recording HUD window bound to `AppController` state. No new dependency or separate overlay framework.
- Keep capture and FFmpeg ownership in existing capture crate. Desktop crate owns presentation and command routing only.
- Extend controller state with enough timing/target metadata for deterministic selector, countdown, recording, stop, cancel, and error transitions.

## Failure Handling

- Invalid/tiny selection remains in selector with visible status.
- Capture/FFmpeg start failure closes transient UI, restores main window, shows exact error, and clears child process/partial output.
- Stop is idempotent. Repeated click/hotkey cannot start another finalization.

## Acceptance

- Window: hover, named highlight, click, saved screenshot, clipboard populated.
- Video: region and window selection, visible countdown/HUD, same-hotkey and button stop, playable H.264/AAC MP4.
- GIF: region and window selection, visible countdown/HUD, same-hotkey and button stop, playable 15 FPS GIF.
- Escape/cancel, recovery after errors, single-instance behavior, output folder, and ShareX-equivalent configured hotkeys verified live.
- Automated state/UI/command tests, clippy, build, and manual end-to-end checks pass; no orphan RapidCap FFmpeg remains.
