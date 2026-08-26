use std::fmt;

use rand::{Rng, distr::Alphanumeric};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputNamer {
    suffix: String,
}

impl OutputNamer {
    pub fn random() -> Self {
        let suffix = rand::rng()
            .sample_iter(Alphanumeric)
            .take(10)
            .map(char::from)
            .collect();
        Self { suffix }
    }

    pub fn for_test(suffix: &str) -> Result<Self, NamingError> {
        if suffix.len() != 10 || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(NamingError);
        }
        Ok(Self {
            suffix: suffix.into(),
        })
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
        format!("{process_name}_{}", self.suffix)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamingError;

impl fmt::Display for NamingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("suffix must contain exactly 10 ASCII alphanumeric characters")
    }
}

impl std::error::Error for NamingError {}
