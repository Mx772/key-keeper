//! [`Secret`]: a string that is zeroised on drop and never printed by `Debug`.
//! Used for game keys and API keys.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(Zeroizing::new(s.into()))
    }

    /// The plaintext. Call sites are the only places a key is ever revealed, which keeps them easy to audit.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// Uppercase ASCII alphanumerics only: `"abcde-fghij "` and `"ABCDEFGHIJ"` are the same key.
    /// Used for duplicate detection.
    pub fn normalized(&self) -> String {
        normalize_key(&self.0)
    }

    /// The first dash-separated group stays visible and every other character becomes a bullet:
    /// `ABCDE-FGHIJ-KLMNO` → `ABCDE-•••••-•••••`. A key without dashes shows its first 4 characters.
    pub fn masked(&self) -> String {
        let s = self.0.trim();
        if s.contains('-') {
            s.split('-')
                .enumerate()
                .map(|(i, part)| {
                    if i == 0 {
                        part.to_string()
                    } else {
                        "•".repeat(part.chars().count())
                    }
                })
                .collect::<Vec<_>>()
                .join("-")
        } else {
            let shown: String = s.chars().take(4).collect();
            let hidden = s.chars().count().saturating_sub(4);
            format!("{shown}{}", "•".repeat(hidden))
        }
    }
}

pub fn normalize_key(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Secret::new(s)
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret::new(s)
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        String::deserialize(de).map(Secret::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_reveals() {
        let s = Secret::new("UNDR7-TALE0-00001");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert!(!format!("{:?}", Some(&s)).contains("UNDR7"));
    }

    #[test]
    fn normalization_ignores_case_spacing_and_dashes() {
        assert_eq!(
            Secret::new(" undr7-tale0-00001 ").normalized(),
            "UNDR7TALE000001"
        );
        assert_eq!(
            Secret::new("UNDR7TALE000001").normalized(),
            Secret::new("undr7 - tale0 - 00001").normalized()
        );
    }

    #[test]
    fn masking() {
        assert_eq!(
            Secret::new("ABCDE-FGHIJ-KLMNO").masked(),
            "ABCDE-•••••-•••••"
        );
        assert_eq!(Secret::new("ABCDEFGH").masked(), "ABCD••••");
        assert_eq!(Secret::new("").masked(), "");
    }

    #[test]
    fn serde_is_a_plain_string() {
        let s = Secret::new("KEY-1");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"KEY-1\"");
        let back: Secret = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
