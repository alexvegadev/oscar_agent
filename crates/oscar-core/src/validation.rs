//! Mandatory host-selected proposal gates. This policy is never deserialized
//! from a plan or model response. Snapshot/process gates are a separate layer.
use crate::{error::OscarError, execution::Validators};
use std::collections::BTreeSet;

#[derive(Clone, Default)]
pub struct ValidationPolicy {
    required: BTreeSet<String>,
}
impl ValidationPolicy {
    /// All named validators must run for every worker artifact, regardless of the
    /// task's model-selected kind, format validator, confidence or provider.
    pub fn require_all(names: impl IntoIterator<Item = String>) -> Result<Self, OscarError> {
        let mut required = BTreeSet::new();
        for name in names {
            if required.len() >= 32
                || name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
                || !required.insert(name)
            {
                return Err(OscarError::Config(
                    "invalid or duplicate required validator".into(),
                ));
            }
        }
        Ok(Self { required })
    }
    pub(crate) fn check_registry(&self, validators: &Validators) -> Result<(), OscarError> {
        if self
            .required
            .iter()
            .any(|name| !validators.contains_key(name))
        {
            return Err(OscarError::Config(
                "required host validator is not registered".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn required(&self) -> impl Iterator<Item = &String> {
        self.required.iter()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.required.is_empty()
    }
}
