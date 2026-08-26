use std::{fs, path::Path};

use rapidcap_capture::{AppPaths, SettingsStore};

fn main() -> anyhow::Result<()> {
    let paths = AppPaths::discover()?;
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
