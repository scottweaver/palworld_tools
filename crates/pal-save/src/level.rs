//! The `Level.sav` wire boundary: GVAS parsing and raw character
//! extraction. Nothing here touches [`pal_core`] types — resolution
//! against the pal database happens in [`crate::import`].
//!
//! Palworld's GVAS tree contains maps whose struct-typed keys and
//! values carry no type annotation; the `gvas` crate needs a hint per
//! such path. We seed the community-known set for `Level.sav` and
//! self-discover the rest: a `MissingHint` error names the exact
//! path, so the parser retries with a generic struct hint (falling
//! back to `Guid` when that misparses) until the file reads. Newly
//! discovered hints are reported so they can graduate into the seed
//! list.

use std::collections::HashMap;
use std::io::Cursor;

use gvas::GvasFile;
use gvas::cursor_ext::ReadExt;
use gvas::error::{DeserializeError, Error as GvasError};
use gvas::game_version::GameVersion;
use gvas::properties::Property;
use gvas::properties::array_property::ArrayProperty;
use gvas::properties::map_property::MapProperty;
use gvas::properties::struct_property::{StructProperty, StructPropertyValue};
use gvas::types::map::HashableIndexMap;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("not a Palworld save container (missing PlZ magic)")]
    NotASave,
    #[error("GVAS parse failed: {0}")]
    Gvas(#[from] Box<GvasError>),
    #[error("hint discovery did not converge after {attempts} attempts (last path: {path})")]
    HintDiscovery { attempts: usize, path: String },
    #[error("worldSaveData.CharacterSaveParameterMap not found or not the expected shape")]
    MissingCharacterMap,
}

/// One entry of `CharacterSaveParameterMap`, still in wire terms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawCharacter {
    pub character_id: Option<String>,
    pub gender: Option<String>,
    pub passives: Vec<String>,
    pub is_player: bool,
}

/// Everything extracted from a `Level.sav`.
#[derive(Debug)]
pub struct LevelSave {
    pub characters: Vec<RawCharacter>,
    /// Hint paths discovered at parse time beyond the seed list —
    /// candidates for [`SEED_HINTS`].
    pub discovered_hints: Vec<(String, String)>,
}

/// Community-known struct hints for `Level.sav` paths the parser
/// cannot infer. Discovery handles anything missing here.
const SEED_HINTS: &[(&str, &str)] = &[
    (
        "worldSaveData.StructProperty.CharacterSaveParameterMap.MapProperty.Key.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.CharacterSaveParameterMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.ItemContainerSaveData.MapProperty.Key.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.ItemContainerSaveData.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.CharacterContainerSaveData.MapProperty.Key.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.CharacterContainerSaveData.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.GroupSaveDataMap.MapProperty.Key.StructProperty",
        "Guid",
    ),
    (
        "worldSaveData.StructProperty.GroupSaveDataMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
];

const MAX_DISCOVERY_ATTEMPTS: usize = 64;

/// Parses a full `Level.sav` (container + GVAS) and extracts the
/// character map.
///
/// # Errors
///
/// Fails when the container magic is absent, the GVAS tree cannot be
/// parsed even after hint discovery, or the character map is missing.
pub fn read_level_sav(bytes: &[u8]) -> Result<LevelSave, SaveError> {
    if !crate::looks_like_sav(bytes) {
        return Err(SaveError::NotASave);
    }

    let mut hints: HashMap<String, String> = SEED_HINTS
        .iter()
        .map(|(path, ty)| ((*path).to_owned(), (*ty).to_owned()))
        .collect();
    let mut discovered: Vec<(String, String)> = Vec::new();

    let file = parse_with_discovery(bytes, &mut hints, &mut discovered)?;
    let characters = extract_characters(&file)?;
    Ok(LevelSave {
        characters,
        discovered_hints: discovered,
    })
}

/// Parses with self-discovering hints: on `MissingHint`, retry the
/// new path as a generic struct; if that attempt then fails with a
/// non-hint error, flip the most recent discovery to `Guid` once
/// before giving up on it.
fn parse_with_discovery(
    bytes: &[u8],
    hints: &mut HashMap<String, String>,
    discovered: &mut Vec<(String, String)>,
) -> Result<GvasFile, SaveError> {
    let mut last_discovered: Option<String> = None;
    for _ in 0..MAX_DISCOVERY_ATTEMPTS {
        let mut cursor = Cursor::new(bytes);
        match GvasFile::read_with_hints(&mut cursor, GameVersion::Palworld, hints) {
            Ok(file) => {
                discovered.extend(
                    hints
                        .iter()
                        .filter(|(path, _)| {
                            !SEED_HINTS.iter().any(|(seed, _)| *seed == path.as_str())
                        })
                        .map(|(path, ty)| (path.clone(), ty.clone())),
                );
                discovered.sort();
                return Ok(file);
            }
            Err(GvasError::Deserialize(DeserializeError::MissingHint(_, path, _))) => {
                let path = path.to_string();
                hints.insert(path.clone(), "Struct".to_owned());
                last_discovered = Some(path);
            }
            Err(error) => {
                // A freshly guessed generic struct may have misparsed:
                // retry that one path as a Guid before failing.
                if let Some(path) = last_discovered.take()
                    && hints.get(&path).is_some_and(|ty| ty == "Struct")
                {
                    hints.insert(path, "Guid".to_owned());
                    continue;
                }
                return Err(SaveError::Gvas(Box::new(error)));
            }
        }
    }
    Err(SaveError::HintDiscovery {
        attempts: MAX_DISCOVERY_ATTEMPTS,
        path: last_discovered.unwrap_or_default(),
    })
}

fn extract_characters(file: &GvasFile) -> Result<Vec<RawCharacter>, SaveError> {
    let world = struct_fields(file.properties.get("worldSaveData"))
        .ok_or(SaveError::MissingCharacterMap)?;
    let Some(Property::MapProperty(map)) = first(world, "CharacterSaveParameterMap") else {
        return Err(SaveError::MissingCharacterMap);
    };
    let MapProperty::Properties { value, .. } = map else {
        return Err(SaveError::MissingCharacterMap);
    };

    Ok(value
        .iter()
        .filter_map(|(_, entry)| raw_data_bytes(entry))
        .map(|bytes| decode_character(&bytes))
        .collect())
}

/// The character entry's `RawData` byte payload, when shaped as
/// expected.
fn raw_data_bytes(entry: &Property) -> Option<Vec<u8>> {
    let fields = match entry {
        Property::StructProperty(s) => custom_fields(s),
        _ => None,
    }?;
    match first(fields, "RawData") {
        Some(Property::ArrayProperty(ArrayProperty::Bytes { bytes })) => Some(bytes.clone()),
        _ => None,
    }
}

/// Decodes one character's `RawData` blob: a standard property list
/// holding a `SaveParameter` struct. Undecodable blobs yield a
/// default (skippable) character rather than failing the whole file.
fn decode_character(bytes: &[u8]) -> RawCharacter {
    let mut cursor = Cursor::new(bytes);
    let hints = HashMap::new();
    let custom_versions = HashableIndexMap::default();
    let mut stack: Vec<String> = Vec::new();
    let mut options = gvas::properties::PropertyOptions {
        hints: &hints,
        properties_stack: &mut stack,
        custom_versions: &custom_versions,
    };

    let mut character = RawCharacter::default();
    loop {
        let Ok(name) = cursor.read_string() else {
            return character;
        };
        if name == "None" {
            break;
        }
        let Ok(value_type) = cursor.read_string() else {
            return character;
        };
        let Ok(property) = Property::new(&mut cursor, &value_type, true, &mut options, None) else {
            return character;
        };
        if name == "SaveParameter"
            && let Property::StructProperty(save_parameter) = &property
            && let Some(fields) = custom_fields(save_parameter)
        {
            populate(&mut character, fields);
        }
    }
    character
}

fn populate(character: &mut RawCharacter, fields: &HashableIndexMap<String, Vec<Property>>) {
    character.character_id = match first(fields, "CharacterID") {
        Some(Property::NameProperty(name)) => name.value.clone(),
        Some(Property::StrProperty(name)) => name.value.clone(),
        _ => None,
    };
    character.gender = match first(fields, "Gender") {
        Some(Property::EnumProperty(gender)) => Some(gender.value.clone()),
        Some(Property::NameProperty(gender)) => gender.value.clone(),
        _ => None,
    };
    character.is_player = matches!(
        first(fields, "IsPlayer"),
        Some(Property::BoolProperty(b)) if b.value
    );
    character.passives = match first(fields, "PassiveSkillList") {
        Some(Property::ArrayProperty(array)) => name_array(array),
        _ => Vec::new(),
    };
}

fn name_array(array: &ArrayProperty) -> Vec<String> {
    match array {
        ArrayProperty::Properties { properties, .. } => properties
            .iter()
            .filter_map(|property| match property {
                Property::NameProperty(name) => name.value.clone(),
                Property::StrProperty(name) => name.value.clone(),
                _ => None,
            })
            .collect(),
        ArrayProperty::Strings { strings, .. } => strings.iter().filter_map(Clone::clone).collect(),
        _ => Vec::new(),
    }
}

fn struct_fields(property: Option<&Property>) -> Option<&HashableIndexMap<String, Vec<Property>>> {
    match property {
        Some(Property::StructProperty(s)) => custom_fields(s),
        _ => None,
    }
}

fn custom_fields(s: &StructProperty) -> Option<&HashableIndexMap<String, Vec<Property>>> {
    match &s.value {
        StructPropertyValue::CustomStruct(fields) => Some(fields),
        _ => None,
    }
}

fn first<'a>(
    fields: &'a HashableIndexMap<String, Vec<Property>>,
    name: &str,
) -> Option<&'a Property> {
    fields.get(name)?.first()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing_requires_the_plz_magic() {
        assert!(!crate::looks_like_sav(b"species = \"Lamball\""));
        assert!(!crate::looks_like_sav(&[0u8; 8]));
        let mut sav = vec![0u8; 16];
        sav[8..11].copy_from_slice(b"PlZ");
        assert!(crate::looks_like_sav(&sav));
    }

    #[test]
    fn non_saves_are_rejected_before_parsing() {
        assert!(matches!(
            read_level_sav(b"not a save at all, definitely toml"),
            Err(SaveError::NotASave)
        ));
    }

    #[test]
    fn corrupt_container_fails_loudly_not_by_panic() {
        let mut sav = vec![0u8; 64];
        sav[8..11].copy_from_slice(b"PlZ");
        sav[11] = 0x31;
        assert!(matches!(
            read_level_sav(&sav),
            Err(SaveError::Gvas(_) | SaveError::HintDiscovery { .. })
        ));
    }
}
