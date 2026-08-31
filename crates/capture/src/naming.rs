use chrono::{DateTime, Local};
use rand::{Rng, distr::Alphanumeric};

/// `2026-08-27_14-32-05`. Sorts chronologically as plain text, and every
/// separator is legal in a Windows filename — `:` is not, so the time uses `-`.
const STAMP: &str = "%Y-%m-%d_%H-%M-%S";

/// Length of the collision guard. Four alphanumerics is one chance in fourteen
/// million of a clash, and only between two captures that land in the same
/// second of the same window.
const GUARD: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputNamer {
    stamp: String,
}

impl OutputNamer {
    /// The stamp for a capture taken at `when`.
    ///
    /// The timestamp leads so Explorer sorts captures in the order they were
    /// taken with no effort from the user, and so "the screenshot from Tuesday"
    /// is findable a week later. The guard is what keeps a second capture of
    /// the same window inside the same second from overwriting the first.
    pub fn at(when: DateTime<Local>) -> Self {
        let guard: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(GUARD)
            .map(char::from)
            .collect();
        Self {
            stamp: format!("{}_{guard}", when.format(STAMP)),
        }
    }

    /// Pins the stamp so a test can assert an exact filename.
    pub fn for_test(stamp: &str) -> Self {
        Self {
            stamp: stamp.into(),
        }
    }

    pub fn file_stem(&self, process_name: &str) -> String {
        let process_name: String = process_name
            .chars()
            .map(|character| match character {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                other => other,
            })
            .collect();
        let process_name = process_name.trim_matches([' ', '.']);
        let process_name = if process_name.is_empty() {
            "Screen"
        } else {
            process_name
        };
        format!("{process_name}_{}", self.stamp)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    #[test]
    fn a_stamp_leads_with_the_capture_time() {
        let when = Local.with_ymd_and_hms(2026, 8, 27, 14, 32, 5).unwrap();
        let stem = OutputNamer::at(when).file_stem("Telegram");

        let guard = stem
            .strip_prefix("Telegram_2026-08-27_14-32-05_")
            .expect("the timestamp has to lead, and it has to be zero-padded");
        assert_eq!(guard.len(), GUARD);
        assert!(guard.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }

    #[test]
    fn two_captures_in_the_same_second_do_not_share_a_name() {
        let when = Local.with_ymd_and_hms(2026, 8, 27, 14, 32, 5).unwrap();
        // 62^4 combinations, so a run of identical draws means the guard is not
        // being redrawn per capture rather than that we got unlucky.
        let names: std::collections::HashSet<_> = (0..64)
            .map(|_| OutputNamer::at(when).file_stem("Screen"))
            .collect();
        assert!(names.len() > 1, "every capture in a second took one name");
    }

    #[test]
    fn a_nameless_process_falls_back_to_screen() {
        assert_eq!(
            OutputNamer::for_test("2026-08-27_14-32-05_a7Kq").file_stem("  ."),
            "Screen_2026-08-27_14-32-05_a7Kq"
        );
    }

    #[test]
    fn characters_windows_rejects_become_underscores() {
        assert_eq!(
            OutputNamer::for_test("0000").file_stem("a<b>c:d\"e/f\\g|h?i*j"),
            "a_b_c_d_e_f_g_h_i_j_0000"
        );
    }
}
