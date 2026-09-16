use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single user-defined station from `$XDG_CONFIG_HOME/gtm/radios.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomRadioStation {
    pub name: String,
    pub url: String,
    /// Optional Radio Browser `stationuuid`, set when a directory station is
    /// saved from the TUI. Lets the daemon re-resolve the playable URL and the
    /// station favicon even if the custom entry lives in `radios.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

/// Absolute path of the custom stations file (`radios.toml`) in the gtm
/// config directory, next to `secret/secrets`.
pub fn custom_radios_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".config"))
            .unwrap_or_else(|| PathBuf::from(".config"))
    });
    base.join("gtm").join("radios.toml")
}

/// Index of a `custom:N` station id (1-based) if `id` has that shape.
pub fn parse_custom_id(id: &str) -> Option<usize> {
    id.strip_prefix("custom:").and_then(|n| n.parse().ok())
}

/// The `[[station]]` table array this crate reads and writes.
#[derive(Default, Deserialize, Serialize)]
struct Store {
    #[serde(default)]
    station: Vec<CustomRadioStation>,
}

/// List every custom station, in file order (index = position + 1).
pub fn list_custom_stations() -> Result<Vec<CustomRadioStation>, String> {
    let path = custom_radios_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => toml::from_str::<Store>(&contents)
            .map(|store| store.station)
            .map_err(|e| format!("radios.toml: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("radios.toml: {e}")),
    }
}

/// The custom station at 1-based `index`, if any.
pub fn station_by_index(index: usize) -> Result<Option<CustomRadioStation>, String> {
    if index == 0 {
        return Ok(None);
    }
    Ok(list_custom_stations()?.get(index - 1).cloned())
}

/// Append a station (optionally carrying its Radio Browser `uuid`) and return
/// its 1-based index.
pub fn add_custom_station(name: &str, url: &str, uuid: Option<&str>) -> Result<usize, String> {
    let mut stations = list_custom_stations()?;
    stations.push(CustomRadioStation {
        name: name.to_string(),
        url: url.to_string(),
        uuid: uuid.map(str::to_string),
    });
    write_store(&stations)?;
    Ok(stations.len())
}

/// Remove a station selected by 1-based index or exact name.
pub fn remove_custom_station(selector: &str) -> Result<CustomRadioStation, String> {
    let mut stations = list_custom_stations()?;
    let index = selector
        .parse::<usize>()
        .ok()
        .and_then(|i| (i >= 1 && i <= stations.len()).then_some(i - 1))
        .or_else(|| stations.iter().position(|s| s.name == selector))
        .ok_or_else(|| format!("no custom station matches {selector:?}"))?;
    let removed = stations.remove(index);
    write_store(&stations)?;
    Ok(removed)
}

fn write_store(stations: &[CustomRadioStation]) -> Result<(), String> {
    let path = custom_radios_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("radios.toml: {e}"))?;
    }
    let store = Store {
        station: stations.to_vec(),
    };
    let contents = toml::to_string(&store).map_err(|e| format!("radios.toml: {e}"))?;
    std::fs::write(&path, contents).map_err(|e| format!("radios.toml: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_id_index() {
        assert_eq!(parse_custom_id("custom:3"), Some(3));
        assert_eq!(parse_custom_id("custom:0"), Some(0));
        assert_eq!(parse_custom_id("custom:"), None);
        assert_eq!(parse_custom_id("custom:abc"), None);
        assert_eq!(parse_custom_id("abc123"), None);
    }

    #[test]
    fn store_toml_roundtrip() {
        let store = Store {
            station: vec![
                CustomRadioStation {
                    name: "SomaFM".into(),
                    url: "http://example.com/soma".into(),
                    uuid: None,
                },
                CustomRadioStation {
                    name: "KEXP".into(),
                    url: "http://example.com/kexp".into(),
                    uuid: Some("abc-123".into()),
                },
            ],
        };
        let s = toml::to_string(&store).unwrap();
        let back: Store = toml::from_str(&s).unwrap();
        assert_eq!(back.station, store.station);
        assert!(s.contains("[[station]]"));
    }

    #[test]
    fn store_empty_roundtrip() {
        let s = toml::to_string(&Store::default()).unwrap();
        let back: Store = toml::from_str(&s).unwrap();
        assert!(back.station.is_empty());
    }

    #[test]
    fn legacy_station_without_uuid_parses() {
        // radios.toml files written before `uuid` existed must keep loading.
        let s = "[[station]]\nname = \"KEXP\"\nurl = \"http://example.com/kexp\"\n";
        let back: Store = toml::from_str(s).unwrap();
        assert_eq!(back.station.len(), 1);
        assert_eq!(back.station[0].uuid, None);
    }
}
