//! App preferences: appearance and the safety limits applied to every query.
//!
//! Stored per section under `app_settings` keys. Fields added later fall back to their defaults
//! (`#[serde(default)]`), and values outside their range are rejected on save.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::errors::AppError;

/// Storage keys, one per section.
pub mod keys {
    pub const APPEARANCE: &str = "appearance";
    pub const SAFETY: &str = "safety";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub theme: ThemePreference,
}

/// Limits applied to every read, manual or agent-driven. Connections are read-only by default
/// regardless of these settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(default, rename_all = "camelCase")]
pub struct SafetySettings {
    /// Most rows loaded per page of table data or query results.
    pub max_rows_per_page: u32,
    /// Most result data kept in memory for one query, in megabytes.
    pub max_result_megabytes: u32,
    /// PostgreSQL `statement_timeout` for each statement, in seconds.
    pub statement_timeout_secs: u32,
    /// PostgreSQL `lock_timeout` for each statement, in seconds.
    pub lock_timeout_secs: u32,
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            max_rows_per_page: 500,
            max_result_megabytes: 16,
            statement_timeout_secs: 30,
            lock_timeout_secs: 5,
        }
    }
}

/// Allowed ranges, shared with the UI through the error messages and tests.
pub mod limits {
    use std::ops::RangeInclusive;

    pub const MAX_ROWS_PER_PAGE: RangeInclusive<u32> = 10..=10_000;
    pub const MAX_RESULT_MEGABYTES: RangeInclusive<u32> = 1..=256;
    pub const STATEMENT_TIMEOUT_SECS: RangeInclusive<u32> = 1..=3_600;
    pub const LOCK_TIMEOUT_SECS: RangeInclusive<u32> = 1..=300;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field} must be between {min} and {max}")]
pub struct OutOfRange {
    field: &'static str,
    min: u32,
    max: u32,
}

impl From<OutOfRange> for AppError {
    fn from(error: OutOfRange) -> Self {
        Self::invalid_input(format!("{error}."))
    }
}

fn check(value: u32, range: RangeInclusive<u32>, field: &'static str) -> Result<(), OutOfRange> {
    if range.contains(&value) {
        Ok(())
    } else {
        Err(OutOfRange {
            field,
            min: *range.start(),
            max: *range.end(),
        })
    }
}

impl SafetySettings {
    /// # Errors
    ///
    /// Returns the first value outside its allowed range.
    pub fn validate(&self) -> Result<(), OutOfRange> {
        check(
            self.max_rows_per_page,
            limits::MAX_ROWS_PER_PAGE,
            "Rows per page",
        )?;
        check(
            self.max_result_megabytes,
            limits::MAX_RESULT_MEGABYTES,
            "Maximum result size",
        )?;
        check(
            self.statement_timeout_secs,
            limits::STATEMENT_TIMEOUT_SECS,
            "Statement timeout",
        )?;
        check(
            self.lock_timeout_secs,
            limits::LOCK_TIMEOUT_SECS,
            "Lock timeout",
        )?;
        if self.lock_timeout_secs > self.statement_timeout_secs {
            return Err(OutOfRange {
                field: "Lock timeout",
                min: *limits::LOCK_TIMEOUT_SECS.start(),
                max: self.statement_timeout_secs,
            });
        }
        Ok(())
    }
}

/// Every settings section.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub safety: SafetySettings,
}

/// Sections to replace; omitted sections stay as they are.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, optional_fields))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub appearance: Option<AppearanceSettings>,
    pub safety: Option<SafetySettings>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_within_their_own_limits() {
        assert_eq!(SafetySettings::default().validate(), Ok(()));
        assert_eq!(
            AppSettings::default().appearance.theme,
            ThemePreference::System
        );
    }

    #[test]
    fn out_of_range_values_name_the_field_and_range() {
        let settings = SafetySettings {
            max_rows_per_page: 5,
            ..SafetySettings::default()
        };

        let error = settings.validate().expect_err("too few rows");

        assert_eq!(
            error.to_string(),
            "Rows per page must be between 10 and 10000"
        );
    }

    #[test]
    fn lock_timeout_cannot_exceed_statement_timeout() {
        let settings = SafetySettings {
            statement_timeout_secs: 10,
            lock_timeout_secs: 20,
            ..SafetySettings::default()
        };

        assert_eq!(
            settings.validate().map_err(|e| e.to_string()),
            Err("Lock timeout must be between 1 and 10".to_owned())
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let stored: SafetySettings =
            serde_json::from_str(r#"{"maxRowsPerPage": 1000}"#).expect("parses");

        assert_eq!(stored.max_rows_per_page, 1000);
        assert_eq!(stored.statement_timeout_secs, 30);
    }

    #[test]
    fn patches_reject_unknown_sections() {
        let result = serde_json::from_str::<SettingsPatch>(r#"{"telemetry": {"enabled": true}}"#);

        assert!(result.is_err());
    }

    /// ts-rs exports types only, so the defaults and ranges are written next to them here. The UI
    /// validates with the same numbers, and CI fails if the committed copy is stale.
    #[test]
    fn export_safety_defaults_and_limits() {
        let defaults = serde_json::to_value(SafetySettings::default()).expect("serializes");
        let fields = [
            ("maxRowsPerPage", limits::MAX_ROWS_PER_PAGE),
            ("maxResultMegabytes", limits::MAX_RESULT_MEGABYTES),
            ("statementTimeoutSecs", limits::STATEMENT_TIMEOUT_SECS),
            ("lockTimeoutSecs", limits::LOCK_TIMEOUT_SECS),
        ];
        let mut names: Vec<&str> = defaults
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        let mut expected: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
        expected.sort_unstable();
        assert_eq!(names, expected, "every safety field needs a limit");

        let mut ts = String::from(
            "// This file was generated by `cargo test` from src-tauri/src/domain/settings.rs. \
             Do not edit this file manually.\nimport type { SafetySettings } from \"./SafetySettings\";\n\n\
             export const SAFETY_DEFAULTS: SafetySettings = {\n",
        );
        for (name, _) in &fields {
            ts.push_str(&format!("  {name}: {},\n", defaults[name]));
        }
        ts.push_str("};\n\n/**\n * Allowed range of each safety setting, inclusive.\n */\nexport const SAFETY_LIMITS = {\n");
        for (name, range) in &fields {
            ts.push_str(&format!(
                "  {name}: {{ min: {}, max: {} }},\n",
                range.start(),
                range.end()
            ));
        }
        ts.push_str("} as const;\n");

        let dir = std::env::var_os("TS_RS_EXPORT_DIR").expect("set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("SafetyLimits.ts"), ts).expect("writes");
    }
}
