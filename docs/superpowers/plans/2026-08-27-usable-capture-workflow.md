# Usable Capture Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add visible window/region selection and unmistakable countdown/recording/stop UI.

**Architecture:** Existing GPUI selector handles drag regions and hover-selected top-level windows. Existing main window becomes compact recording HUD; four narrow GPUI windows mark selected boundary. Existing controller/FFmpeg path remains owner of lifecycle.

**Tech Stack:** Rust 1.97.1, GPUI, windows-rs, existing capture/FFmpeg crates.

## Global Constraints

- No new dependencies.
- Preserve ShareX-derived output/encoder/hotkey settings.
- Window, Video, GIF must expose target, countdown, recording state, and stop control.
- No orphan FFmpeg or partial finalized output.

---

### Task 1: Window target discovery

**Files:** Modify `crates/desktop/src/platform.rs`; test same file.

**Interfaces:** Produce `window_target_at(point: (i32, i32)) -> anyhow::Result<CaptureTarget>`.

- [ ] Add failing deterministic tests for point containment and same-process/excluded candidate filtering.
- [ ] Run `rustup run 1.97.1 cargo test -p rapidcap-desktop platform::tests --target-dir C:\Users\inspire\Documents\Codex\2026-08-27\new-chat\work\rapidcap-target-fix`; expect new tests fail before implementation.
- [ ] Implement z-order enumeration, visible top-level bounds, process name, current-process exclusion.
- [ ] Rerun focused tests; expect pass.
- [ ] Commit `feat: select visible window under pointer`.

### Task 2: Shared selector

**Files:** Modify `crates/desktop/src/overlay.rs`, `crates/desktop/src/main.rs`; test `overlay.rs`.

**Interfaces:** Selector emits `CaptureTarget::Window` on click and `CaptureTarget::Region` after drag.

- [ ] Add failing tests for click-versus-drag selection and selector labels.
- [ ] Run focused overlay tests; expect fail.
- [ ] Add hover target, highlighted window bounds/name, drag threshold, Window-only click routing; route Window/Video/GIF through selector.
- [ ] Rerun focused tests; expect pass.
- [ ] Commit `feat: add ShareX-style target selector`.

### Task 3: Recording boundary and HUD

**Files:** Modify `crates/desktop/src/overlay.rs`, `crates/desktop/src/controller.rs`, `crates/desktop/src/window.rs`, `crates/desktop/src/main.rs`; tests beside code.

**Interfaces:** Produce `open_recording_border(cx, target)`, controller elapsed/countdown presentation, Stop/Cancel actions.

- [ ] Add failing tests for target region conversion, countdown labels, elapsed labels, and stop/cancel transitions.
- [ ] Run focused desktop tests; expect fail.
- [ ] Open four non-focused red border windows after selection; show main HUD with target and countdown; refresh timer every second; close boundary after finalization/error/cancel.
- [ ] Make matching start hotkey stop active recording; Escape cancels selection/countdown; Stop finalizes once.
- [ ] Rerun focused tests; expect pass.
- [ ] Commit `feat: show recording boundary timer and controls`.

### Task 4: End-to-end verification

**Files:** No production changes unless a reproduced acceptance failure requires TDD fix.

- [ ] Run full workspace tests, clippy with `-D warnings`, and dev build.
- [ ] Live-test Window hover/click screenshot and clipboard.
- [ ] Live-test Video and GIF selector, countdown, boundary/HUD, button and hotkey stop; verify playable outputs with `ffprobe.exe`.
- [ ] Verify Escape/cancel, one RapidCap instance, restored ShareX, clean Git, and zero RapidCap-owned FFmpeg.
