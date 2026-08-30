use std::path::Path;

use anyhow::{Context, bail};
use serde::de::DeserializeOwned;

pub fn read<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        Some("toml") => {
            toml::from_str(&contents).with_context(|| format!("invalid TOML in {}", path.display()))
        }
        _ => bail!(
            "document must use a .json or .toml extension: {}",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::read;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Example {
        value: u32,
    }

    #[test]
    fn reads_supported_formats_and_rejects_ambiguous_extensions() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let json = directory.path().join("example.json");
        let toml = directory.path().join("example.toml");
        let ambiguous = directory.path().join("example.txt");
        std::fs::write(&json, r#"{"value": 7}"#).expect("JSON fixture");
        std::fs::write(&toml, "value = 8").expect("TOML fixture");
        std::fs::write(&ambiguous, "value = 9").expect("ambiguous fixture");

        assert_eq!(read::<Example>(&json).expect("JSON document").value, 7);
        assert_eq!(read::<Example>(&toml).expect("TOML document").value, 8);
        assert!(read::<Example>(&ambiguous).is_err());
    }
}
