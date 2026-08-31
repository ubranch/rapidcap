#![cfg_attr(not(test), windows_subsystem = "windows")]

mod controller;
mod icons;
mod motion;
mod overlay;
mod platform;
mod theme;
mod tray;
mod window;

use std::{
    cell::RefCell,
    fs,
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use gpui::{App, AppContext as _};
use gpui_platform::application;
use rapidcap_capture::{
    AppPaths, CaptureCommand, CaptureKind, CaptureState, CaptureTarget, RecordingSession,
    SettingsStore, capture_and_save, write_clipboard, write_clipboard_file,
};

#[cfg(target_os = "macos")]
use crate::window::close_on_exit_request;
use crate::{
    controller::AppController,
    icons::IconAssets,
    overlay::{close_recording_hud, open_recording_hud, open_region_overlay, overlay_key_bindings},
    platform::{
        PlatformEvent, PlatformRuntime, SingleInstance, hide_main_window, hide_recording_frame,
        open_path, probe_payload, show_main_window, show_recording_frame,
    },
    window::{key_bindings, open_main_window},
};

/// The macOS menu bar, which exists only so Command-W and Command-Q work.
///
/// AppKit gives an application's main menu first refusal on every Command
/// chord, before the key window is offered the event. A GPUI key binding for
/// `cmd-q` alone therefore never fires - the menu swallows the chord and finds
/// nothing to run - and an app with no menu at all cannot be quit or hidden
/// from the keyboard, which is what happened here. Naming the two actions in a
/// menu is what wires the chords up, and it also puts them somewhere a user can
/// find them.
///
/// Windows has no such menu and gets its close from Alt+F4, so this is
/// macOS-only rather than a shared surface with an empty implementation.
#[cfg(target_os = "macos")]
fn install_app_menu(cx: &mut App, controller: &gpui::Entity<AppController>) {
    cx.on_action(|_: &window::HidePanelAction, _cx| hide_main_window());
    let quit = controller.clone();
    cx.on_action(move |_: &window::QuitAction, cx| close_on_exit_request(&quit, cx));
    cx.set_menus([gpui::Menu {
        name: "RapidCap".into(),
        items: vec![
            gpui::MenuItem::action("Hide RapidCap", window::HidePanelAction),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Quit RapidCap", window::QuitAction),
        ],
        disabled: false,
    }]);
}

#[cfg(not(target_os = "macos"))]
fn install_app_menu(_cx: &mut App, _controller: &gpui::Entity<AppController>) {}

enum RecordingControl {
    Stop,
    Pause,
    Resume,
}

fn main() -> anyhow::Result<()> {
    let paths = AppPaths::discover()?;
    if std::env::args_os().any(|argument| argument == "--probe") {
        println!("{}", probe_payload(&paths.capture_root));
        return Ok(());
    }

    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };
    fs::create_dir_all(&paths.log_dir)?;
    fs::create_dir_all(&paths.capture_root)?;
    prune_logs(&paths.log_dir, 7)?;

    let file = tracing_appender::rolling::daily(&paths.log_dir, "rapidcap.log");
    let (writer, _log_guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .try_init()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let settings = SettingsStore::new(paths.settings_file.clone()).load()?;
    tracing::info!(
        schema_version = settings.schema_version,
        output = %paths.capture_root.display(),
        "RapidCap startup"
    );

    let silent = std::env::args_os().any(|argument| argument == "--silent");
    application().with_assets(IconAssets).run(move |cx: &mut App| {
        let mut bindings = key_bindings();
        bindings.extend(overlay_key_bindings());
        cx.bind_keys(bindings);
        let controller = cx.new(|_| AppController::new(settings, paths));
        install_app_menu(cx, &controller);
        let recording_stop = Arc::new(Mutex::new(None::<mpsc::Sender<RecordingControl>>));
        let recording_hud = Rc::new(RefCell::new(None));
        let main_window =
            open_main_window(cx, controller.clone(), !silent).expect("open RapidCap main window");
        let recording_window = main_window;
        cx.subscribe(&controller, {
            let recording_stop = recording_stop.clone();
            let recording_hud = recording_hud.clone();
            move |controller, command, cx| match command {
                CaptureCommand::CaptureRegion
                | CaptureCommand::CaptureActiveWindow
                | CaptureCommand::ToggleVideo
                | CaptureCommand::ToggleGif
                    if matches!(controller.read(cx).state(), CaptureState::Selecting(_)) =>
                {
                    let _ = main_window.update(cx, |_view, window, _cx| {
                        window.minimize_window();
                    });
                    if let Err(error) = open_region_overlay(cx, controller.clone()) {
                        tracing::error!(%error, "open region overlay");
                        let _ = controller.update(cx, |controller, cx| {
                            controller.dispatch(CaptureCommand::Cancel, cx)
                        });
                        let _ = main_window.update(cx, |_view, window, _cx| {
                            window.activate_window();
                        });
                    }
                }
                CaptureCommand::ToggleVideo | CaptureCommand::ToggleGif
                    if matches!(controller.read(cx).state(), CaptureState::Finalizing(_)) =>
                {
                    if let Some(sender) = recording_stop.lock().unwrap().take() {
                        let _ = sender.send(RecordingControl::Stop);
                    }
                }
                CaptureCommand::TogglePause => {
                    if let Some(sender) = recording_stop.lock().unwrap().as_ref() {
                        let control = if matches!(controller.read(cx).state(), CaptureState::Paused(_)) {
                            RecordingControl::Pause
                        } else {
                            RecordingControl::Resume
                        };
                        let _ = sender.send(control);
                    }
                }
                CaptureCommand::ToggleVideo | CaptureCommand::ToggleGif
                    if matches!(controller.read(cx).state(), CaptureState::Idle) =>
                {
                    if let Some(sender) = recording_stop.lock().unwrap().take() {
                        let _ = sender.send(RecordingControl::Stop);
                    }
                    hide_recording_frame();
                    close_recording_hud(&mut recording_hud.borrow_mut(), cx);
                    show_main_window();
                    let _ = main_window.update(cx, |_view, window, _cx| {
                        window.activate_window();
                    });
                }
                CaptureCommand::Cancel => {
                    if let Some(sender) = recording_stop.lock().unwrap().take() {
                        let _ = sender.send(RecordingControl::Stop);
                    }
                    hide_recording_frame();
                    close_recording_hud(&mut recording_hud.borrow_mut(), cx);
                    show_main_window();
                    let _ = main_window.update(cx, |_view, window, _cx| {
                        window.activate_window();
                    });
                }
                _ => {}
            }
        })
        .detach();
        cx.subscribe(&controller, {
            let recording_stop = recording_stop.clone();
            let recording_hud = recording_hud.clone();
            move |controller, target: &CaptureTarget, cx| {
            let state = controller.read(cx).state().clone();
            let (settings, paths) = controller.read_with(cx, |controller, _| {
                (controller.settings().clone(), controller.paths().clone())
            });
            let target = target.clone();
            match state {
                CaptureState::Selecting(
                    CaptureKind::RegionScreenshot | CaptureKind::ActiveWindowScreenshot,
                ) => {
                    let task = cx.background_executor().spawn(async move {
                        std::thread::sleep(Duration::from_millis(40));
                        let saved = capture_and_save(&target, &settings, &paths)?;
                        let copied = match write_clipboard(&saved) {
                            Ok(()) => true,
                            Err(error) => {
                                tracing::warn!(%error, path = %saved.path.display(), "clipboard write failed after screenshot save");
                                false
                            }
                        };
                        Ok((saved, copied))
                    });
                    cx.spawn(async move |cx| {
                        let (result, copied) = match task.await {
                            Ok((saved, copied)) => (Ok(saved), copied),
                            Err(error) => (Err(error), false),
                        };
                        controller.update(cx, |controller, cx| {
                            controller.finish_screenshot(result, copied, cx)
                        });
                        // Opening the overlay minimised the panel, and
                        // `activate_window` only raises and focuses - it cannot
                        // un-minimise. Without this the panel simply never came
                        // back after a screenshot.
                        show_main_window();
                        let _ = recording_window.update(cx, |_view, window, _cx| {
                            window.activate_window();
                        });
                        cx.update(|cx| cx.activate(true));
                    })
                    .detach();
                }
                CaptureState::Countdown(kind @ (CaptureKind::Video | CaptureKind::Gif), seconds) => {
                    hide_recording_frame();
                    close_recording_hud(&mut recording_hud.borrow_mut(), cx);
                    match &target {
                        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => {
                            show_recording_frame(region, theme::FRAME)
                        }
                    }
                    match open_recording_hud(cx, controller.clone(), target.clone()) {
                        Ok(handle) => *recording_hud.borrow_mut() = Some(handle),
                        Err(error) => {
                            tracing::error!(%error, "open recording HUD");
                            hide_recording_frame();
                            let _ = controller.update(cx, |controller, cx| {
                                controller.dispatch(CaptureCommand::Cancel, cx)
                            });
                            return;
                        }
                    }
                    hide_main_window();
                    cx.activate(true);
                    let (stop_sender, stop_receiver) = mpsc::channel();
                    *recording_stop.lock().unwrap() = Some(stop_sender);
                    let (started_sender, started_receiver) = async_channel::bounded(1);
                    let task = cx.background_executor().spawn(async move {
                        match stop_receiver.recv_timeout(Duration::from_secs(u64::from(seconds))) {
                            Ok(_) => return Ok(None),
                            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                        let mut session = match RecordingSession::start(kind, &target, &settings, &paths)
                        {
                            Ok(session) => session,
                            Err(error) => {
                                let _ = started_sender.send_blocking(Err(error.clone()));
                                return Err(error);
                            }
                        };
                        let _ = started_sender.send_blocking(Ok(()));
                        loop {
                            match stop_receiver.recv() {
                                Ok(RecordingControl::Pause) => session.pause()?,
                                Ok(RecordingControl::Resume) => session.resume()?,
                                Ok(RecordingControl::Stop) | Err(_) => break session.stop().map(Some),
                            }
                        }
                    });
                    let recording_stop = recording_stop.clone();
                            let recording_hud = recording_hud.clone();
                    cx.spawn(async move |cx| {
                        match started_receiver.recv().await {
                            Ok(Ok(())) => controller.update(cx, |controller, cx| {
                                controller.begin_recording(kind, cx)
                            }),
                            Ok(Err(error)) => {
                                controller.update(cx, |controller, cx| {
                                    controller.finish_recording(Err(error), false, cx)
                                });
                                hide_recording_frame();
                                close_recording_hud(&mut recording_hud.borrow_mut(), cx);
                                show_main_window();
                                return;
                            }
                            Err(_) => {
                                hide_recording_frame();
                                close_recording_hud(&mut recording_hud.borrow_mut(), cx);
                                show_main_window();
                                return;
                            }
                        }
                        let result = task.await.and_then(|path| {
                            path.ok_or_else(|| unreachable!("started recording cannot be cancelled"))
                        });
                        // A finished recording belongs on the clipboard for the
                        // same reason a screenshot does: the next thing you do
                        // with it is paste it somewhere.
                        let copied = match &result {
                            Ok(path) => match write_clipboard_file(path) {
                                Ok(()) => true,
                                Err(error) => {
                                    tracing::warn!(%error, path = %path.display(), "clipboard write failed after recording save");
                                    false
                                }
                            },
                            Err(_) => false,
                        };
                        recording_stop.lock().unwrap().take();
                        controller.update(cx, |controller, cx| {
                            controller.finish_recording(result, copied, cx)
                        });
                        hide_recording_frame();
                        close_recording_hud(&mut recording_hud.borrow_mut(), cx);
                        show_main_window();
                        let _ = recording_window.update(cx, |_view, window, _cx| {
                            window.activate_window();
                        });
                    })
                    .detach();
                }
                _ => {}
            }
            }
        })
        .detach();
        let runtime = match PlatformRuntime::start() {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::error!(%error, "start RapidCap Windows integration");
                cx.quit();
                return;
            }
        };
        let events = runtime.receiver();
        // Read before the runtime moves into the global. A chord another app
        // owns fails silently at the OS level, so the panel is the only place
        // the user can find out - and it opened before this ran.
        let unavailable = runtime.unavailable_hotkeys().to_vec();
        cx.set_global(runtime);
        if !unavailable.is_empty() {
            let _ = main_window.update(cx, |view, _window, cx| {
                view.set_unavailable_hotkeys(unavailable, cx)
            });
        }
        // The tray is the only RapidCap surface on screen while the panel is
        // minimised and the HUD sits over some other part of the display, so it
        // has to carry the capture state rather than stay a static badge.
        cx.observe(&controller, |controller, cx| {
            let state = controller.read(cx).state().clone();
            cx.global::<PlatformRuntime>().show_capture_state(&state);
        })
        .detach();
        cx.spawn(async move |cx| {
            while let Ok(event) = events.recv().await {
                match event {
                    PlatformEvent::Capture(command) => {
                        controller.update(cx, |controller, cx| {
                            let _ = controller.dispatch(command, cx);
                        });
                    }
                    PlatformEvent::Show => {
                        // The panel is hidden with `ShowWindow(SW_HIDE)`, and
                        // `activate_window` cannot bring back a window that is
                        // not visible - that is why "Show" in the tray menu did
                        // nothing at all.
                        show_main_window();
                        let _ = main_window.update(cx, |_view, window, _cx| {
                            window.activate_window();
                        });
                        cx.update(|cx| cx.activate(true));
                    }
                    PlatformEvent::OpenOutput => {
                        let output = controller
                            .read_with(cx, |controller, _| controller.paths().capture_root.clone());
                        if let Err(error) = open_path(&output) {
                            tracing::error!(%error, "open output folder");
                        }
                    }
                    PlatformEvent::Exit => {
                        // Same rule as the panel's close button, from the same
                        // predicate: a live capture holds the app open, an error
                        // does not. Guarding on `Idle` instead meant a failed
                        // capture made Exit do nothing at all.
                        let blocked = controller
                            .read_with(cx, |controller, _| controller.state().blocks_exit());
                        if !blocked {
                            cx.update(|cx| cx.quit());
                        }
                    }
                }
            }
        })
        .detach();
        if !silent {
            cx.activate(true);
        }
    });
    Ok(())
}

fn prune_logs(directory: &Path, keep: usize) -> std::io::Result<()> {
    let mut logs: Vec<_> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("rapidcap.log")
        })
        .collect();
    logs.sort_by_key(|entry| entry.file_name());
    let remove_count = logs.len().saturating_sub(keep);
    for entry in logs.into_iter().take(remove_count) {
        fs::remove_file(entry.path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_retention_keeps_seven_newest_files() {
        let temp = tempfile::tempdir().unwrap();
        for day in 1..=9 {
            std::fs::write(
                temp.path().join(format!("rapidcap.log.2026-08-{day:02}")),
                b"log",
            )
            .unwrap();
        }
        super::prune_logs(temp.path(), 7).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names.len(), 7);
        assert_eq!(names[0], "rapidcap.log.2026-08-03");
    }
}
