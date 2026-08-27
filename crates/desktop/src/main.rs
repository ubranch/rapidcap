#![cfg_attr(not(test), windows_subsystem = "windows")]

mod controller;
mod platform;
mod window;

use std::{fs, path::Path};

use gpui::{App, AppContext as _};
use gpui_platform::application;
use rapidcap_capture::{AppPaths, CaptureState, SettingsStore};

use crate::{
    controller::AppController,
    platform::{PlatformEvent, PlatformRuntime, SingleInstance, open_folder, probe_payload},
    window::{key_bindings, open_main_window},
};

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
    application().run(move |cx: &mut App| {
        cx.bind_keys(key_bindings());
        let controller = cx.new(|_| AppController::new(settings, paths));
        let main_window = open_main_window(cx, controller.clone(), !silent)
            .expect("open RapidCap main window");
        let runtime = match PlatformRuntime::start() {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::error!(%error, "start RapidCap Windows integration");
                cx.quit();
                return;
            }
        };
        let events = runtime.receiver();
        cx.set_global(runtime);
        cx.spawn(async move |cx| {
            while let Ok(event) = events.recv().await {
                match event {
                    PlatformEvent::Capture(command) => {
                        controller.update(cx, |controller, cx| {
                            let _ = controller.dispatch(command, cx);
                        });
                    }
                    PlatformEvent::Show => {
                        let _ = main_window.update(cx, |_view, window, _cx| {
                            window.activate_window();
                        });
                        cx.update(|cx| cx.activate(true));
                    }
                    PlatformEvent::OpenOutput => {
                        let output = controller
                            .read_with(cx, |controller, _| controller.paths().capture_root.clone());
                        if let Err(error) = open_folder(&output) {
                            tracing::error!(%error, "open output folder");
                        }
                    }
                    PlatformEvent::Exit => {
                        let idle = controller.read_with(cx, |controller, _| {
                            matches!(controller.state(), CaptureState::Idle)
                        });
                        if idle {
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
