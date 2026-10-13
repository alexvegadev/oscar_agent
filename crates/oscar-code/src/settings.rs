use oscar_core::{config::WorkMode, error::OscarError, planning::ProviderPreference};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

/// Non-secret preferences. Provider models and credential references live in core config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub dark: bool,
    pub max_turns: usize,
    pub timeout_ms: u64,
    pub provider: ProviderPreference,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            max_turns: 32,
            timeout_ms: 60_000,
            provider: ProviderPreference::Local,
        }
    }
}

impl Settings {
    pub fn validate(&self, mode: WorkMode) -> Result<(), OscarError> {
        if !(1..=256).contains(&self.max_turns) || !(1..=3_600_000).contains(&self.timeout_ms) {
            return Err(OscarError::Config("desktop limits out of range".into()));
        }
        if matches!(
            (mode, self.provider),
            (WorkMode::Local, ProviderPreference::Remote)
                | (WorkMode::FullRemote, ProviderPreference::Local)
        ) {
            return Err(OscarError::Config(
                "desktop provider conflicts with work_mode".into(),
            ));
        }
        Ok(())
    }

    pub fn load(path: &Path, mode: WorkMode) -> Result<Self, OscarError> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    provider: if mode == WorkMode::FullRemote {
                        ProviderPreference::Remote
                    } else {
                        ProviderPreference::Local
                    },
                    ..Self::default()
                });
            }
            Err(_) => return Err(OscarError::Config("cannot open desktop settings".into())),
        };
        let mut text = String::new();
        file.take(4097)
            .read_to_string(&mut text)
            .map_err(|_| OscarError::Config("cannot read desktop settings".into()))?;
        if text.len() > 4096 {
            return Err(OscarError::Config("desktop settings exceed 4 KiB".into()));
        }
        let settings: Self = toml::from_str(&text)
            .map_err(|_| OscarError::Config("invalid desktop settings".into()))?;
        settings.validate(mode)?;
        Ok(settings)
    }

    /// Saving is an explicit host action. No prompts or credentials are serialized.
    pub fn save(&self, path: &Path, mode: WorkMode) -> Result<(), OscarError> {
        self.validate(mode)?;
        let text = toml::to_string_pretty(self)
            .map_err(|_| OscarError::Config("cannot serialize desktop settings".into()))?;
        std::fs::write(path, text)
            .map_err(|_| OscarError::Config("cannot save desktop settings".into()))
    }
}
