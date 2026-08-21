use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind} identifier {value:?}: {reason}")]
pub struct IdentifierError {
    pub kind: &'static str,
    pub value: String,
    pub reason: &'static str,
}

fn validate_identifier(
    kind: &'static str,
    value: &str,
    allow_dot: bool,
) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError {
            kind,
            value: value.into(),
            reason: "must not be empty",
        });
    }
    if value.starts_with('-') || value.ends_with('-') {
        return Err(IdentifierError {
            kind,
            value: value.into(),
            reason: "must not start or end with '-'",
        });
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || (allow_dot && b == b'.'))
    {
        return Err(IdentifierError {
            kind,
            value: value.into(),
            reason: "contains characters outside [A-Za-z0-9_-] (and '.' where allowed)",
        });
    }
    Ok(())
}

macro_rules! id_newtype {
    ($name:ident, $kind:literal, $dot:expr) => {
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate_identifier($kind, &value, $dot)?;
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = IdentifierError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }
        impl TryFrom<String> for $name {
            type Error = IdentifierError;
            fn try_from(v: String) -> Result<Self, Self::Error> {
                Self::new(v)
            }
        }
        impl TryFrom<&str> for $name {
            type Error = IdentifierError;
            fn try_from(v: &str) -> Result<Self, Self::Error> {
                Self::new(v)
            }
        }
    };
}
id_newtype!(Namespace, "namespace", false);
id_newtype!(PluginKind, "plugin kind", false);
id_newtype!(PluginName, "plugin name", false);
id_newtype!(InterfaceId, "interface", true);
id_newtype!(CapabilityId, "capability", true);

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct PluginId {
    pub namespace: Namespace,
    pub kind: PluginKind,
    pub name: PluginName,
}
impl PluginId {
    pub fn new(
        namespace: impl AsRef<str>,
        kind: impl AsRef<str>,
        name: impl AsRef<str>,
    ) -> Result<Self, IdentifierError> {
        Ok(Self {
            namespace: Namespace::new(namespace.as_ref())?,
            kind: PluginKind::new(kind.as_ref())?,
            name: PluginName::new(name.as_ref())?,
        })
    }
    pub fn executable_name(&self) -> String {
        format!("{}-{}-{}", self.namespace, self.kind, self.name)
    }
    pub fn selector(&self) -> String {
        format!("{}:{}", self.kind, self.name)
    }
}
impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.namespace, self.kind, self.name)
    }
}
