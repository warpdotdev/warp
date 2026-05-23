use std::borrow::Cow;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

/// An application's canonical identifier.
#[derive(Debug, Clone)]
pub struct AppId {
    qualifier: Cow<'static, str>,
    organization: Cow<'static, str>,
    application_name: Cow<'static, str>,
}

impl AppId {
    /// Constructs a new [`AppId`] from its constituent parts.
    pub fn new(
        qualifier: impl Into<Cow<'static, str>>,
        organization: impl Into<Cow<'static, str>>,
        application_name: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            qualifier: qualifier.into(),
            organization: organization.into(),
            application_name: application_name.into(),
        }
    }

    /// Parses an three-component app identifier string (e.g.: com.example.App)
    /// into an [`AppId`].
    pub fn parse(app_id: &str) -> anyhow::Result<Self> {
        let &[qualifier, organization, application_name] =
            app_id.splitn(4, '.').collect_vec().as_slice()
        else {
            anyhow::bail!("App ID does not contain three components, separated by periods.");
        };
        Ok(Self {
            qualifier: Cow::Owned(qualifier.to_owned()),
            organization: Cow::Owned(organization.to_owned()),
            application_name: Cow::Owned(application_name.to_owned()),
        })
    }

    /// Returns the qualifier component of the app ID.
    pub fn qualifier(&self) -> &str {
        &self.qualifier
    }

    /// Returns the organization component of the app ID.
    pub fn organization(&self) -> &str {
        &self.organization
    }

    /// Returns the name of the application.
    pub fn application_name(&self) -> &str {
        &self.application_name
    }
}

impl<'de> Deserialize<'de> for AppId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&str>::deserialize(deserializer)?;
        Self::parse(s).map_err(serde::de::Error::custom)
    }
}

impl Serialize for AppId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl std::fmt::Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}",
            self.qualifier, self.organization, self.application_name
        )
    }
}

#[cfg(test)]
#[path = "app_id_tests.rs"]
mod tests;
