//! The `Level.sav` wire boundary: GVAS parsing and raw character
//! extraction over the decompressed bytes from [`crate::container`].
//! Nothing here touches [`pal_core`] types — resolution against the
//! pal database happens in [`crate::import`].
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
    #[error(transparent)]
    Container(#[from] crate::container::ContainerError),
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
    /// Character-map entries whose shape did not match expectations —
    /// nonzero means the game format moved under us.
    pub malformed_entries: usize,
    /// Distinct decode failures across character blobs, with counts.
    pub decode_issues: Vec<(String, usize)>,
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
    // Graduated from discovery against a real Palworld 0.6-era save
    // (2026-08-03).
    (
        "worldSaveData.StructProperty.BaseCampSaveData.MapProperty.Key.StructProperty",
        "Guid",
    ),
    (
        "worldSaveData.StructProperty.BaseCampSaveData.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.BaseCampSaveData.MapProperty.Value.StructProperty.ModuleMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.DungeonSaveData.ArrayProperty.MapObjectSaveData.ArrayProperty.ConcreteModel.StructProperty.ModuleMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.EnemyCampSaveData.StructProperty.EnemyCampStatusMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.EnemyCampSaveData.StructProperty.EnemyCampStatusMap.MapProperty.Value.StructProperty.TreasureBoxInfoMapBySpawnerName.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.FoliageGridSaveDataMap.MapProperty.Key.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.FoliageGridSaveDataMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.FoliageGridSaveDataMap.MapProperty.Value.StructProperty.ModelMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.FoliageGridSaveDataMap.MapProperty.Value.StructProperty.ModelMap.MapProperty.Value.StructProperty.InstanceDataMap.MapProperty.Key.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.FoliageGridSaveDataMap.MapProperty.Value.StructProperty.ModelMap.MapProperty.Value.StructProperty.InstanceDataMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.GuildExtraSaveDataMap.MapProperty.Key.StructProperty",
        "Guid",
    ),
    (
        "worldSaveData.StructProperty.GuildExtraSaveDataMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.InvaderSaveData.MapProperty.Key.StructProperty",
        "Guid",
    ),
    (
        "worldSaveData.StructProperty.InvaderSaveData.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.LockGimmickSaveData.MapProperty.Key.StructProperty",
        "Guid",
    ),
    (
        "worldSaveData.StructProperty.LockGimmickSaveData.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSaveData.ArrayProperty.ConcreteModel.StructProperty.ModuleMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSaveData.ArrayProperty.Model.StructProperty.EffectMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSpawnerInStageSaveData.MapProperty.Key.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSpawnerInStageSaveData.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSpawnerInStageSaveData.MapProperty.Value.StructProperty.SpawnerDataMapByLevelObjectInstanceId.MapProperty.Key.StructProperty",
        "Guid",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSpawnerInStageSaveData.MapProperty.Value.StructProperty.SpawnerDataMapByLevelObjectInstanceId.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.MapObjectSpawnerInStageSaveData.MapProperty.Value.StructProperty.SpawnerDataMapByLevelObjectInstanceId.MapProperty.Value.StructProperty.ItemMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.OilrigSaveData.StructProperty.OilrigMap.MapProperty.Value.StructProperty",
        "Struct",
    ),
    (
        "worldSaveData.StructProperty.WorkSaveData.ArrayProperty.WorkAssignMap.MapProperty.Value.StructProperty",
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
    let gvas_bytes = crate::container::decompress(bytes)?;

    let mut hints: HashMap<String, String> = SEED_HINTS
        .iter()
        .map(|(path, ty)| ((*path).to_owned(), (*ty).to_owned()))
        .collect();
    let mut discovered: Vec<(String, String)> = Vec::new();

    let file = parse_with_discovery(&gvas_bytes, &mut hints, &mut discovered)?;
    let extraction = extract_characters(&file)?;
    discovered.extend(extraction.nested_hints);
    discovered.sort();
    Ok(LevelSave {
        characters: extraction.characters,
        malformed_entries: extraction.malformed,
        decode_issues: extraction.decode_issues,
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
        match GvasFile::read_with_hints(&mut cursor, GameVersion::Default, hints) {
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

fn extract_characters(file: &GvasFile) -> Result<Extraction, SaveError> {
    let world = struct_fields(file.properties.get("worldSaveData"))
        .ok_or(SaveError::MissingCharacterMap)?;
    let Some(Property::MapProperty(map)) = first(world, "CharacterSaveParameterMap") else {
        return Err(SaveError::MissingCharacterMap);
    };
    let MapProperty::Properties { value, .. } = map else {
        return Err(SaveError::MissingCharacterMap);
    };

    // Vector-like struct widths depend on the save's engine era; the
    // nested blobs must decode under the same custom versions as the
    // outer file or every sized read is wrong.
    let custom_versions = match &file.header {
        gvas::GvasHeader::Version2 {
            custom_versions, ..
        }
        | gvas::GvasHeader::Version3 {
            custom_versions, ..
        } => custom_versions,
    };

    let mut characters = Vec::new();
    let mut malformed = 0usize;
    let mut nested_hints: HashMap<String, String> = HashMap::new();
    let mut issues: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (_, entry) in value {
        match raw_data_bytes(entry) {
            Some(bytes) => match decode_character(bytes, custom_versions, &mut nested_hints) {
                Ok(character) => characters.push(character),
                Err(issue) => *issues.entry(issue).or_default() += 1,
            },
            None => malformed += 1,
        }
    }
    let extraction = Extraction {
        characters,
        malformed,
        decode_issues: issues.into_iter().collect(),
        nested_hints: nested_hints.into_iter().collect(),
    };
    Ok(extraction)
}

struct Extraction {
    characters: Vec<RawCharacter>,
    malformed: usize,
    decode_issues: Vec<(String, usize)>,
    nested_hints: Vec<(String, String)>,
}

/// The character entry's `RawData` byte payload, when shaped as
/// expected. Map values arrive as bare struct values (no header);
/// headered structs are accepted too.
fn raw_data_bytes(entry: &Property) -> Option<&[u8]> {
    let fields = entry_fields(entry)?;
    match first(fields, "RawData") {
        Some(Property::ArrayProperty(ArrayProperty::Bytes { bytes })) => Some(bytes),
        _ => None,
    }
}

fn entry_fields(entry: &Property) -> Option<&HashableIndexMap<String, Vec<Property>>> {
    match entry {
        Property::StructProperty(s) => custom_fields(s),
        Property::StructPropertyValue(StructPropertyValue::CustomStruct(fields)) => Some(fields),
        _ => None,
    }
}

/// Decodes one character's `RawData` blob with the same
/// hint-discovery loop the outer file uses; the nested hint map is
/// shared across all blobs of a save.
fn decode_character(
    bytes: &[u8],
    custom_versions: &HashableIndexMap<gvas::types::Guid, u32>,
    nested_hints: &mut HashMap<String, String>,
) -> Result<RawCharacter, String> {
    let mut last_discovered: Option<String> = None;
    for _ in 0..MAX_DISCOVERY_ATTEMPTS {
        match decode_once(bytes, custom_versions, nested_hints) {
            Ok(character) => return Ok(character),
            Err(GvasError::Deserialize(DeserializeError::MissingHint(_, path, _))) => {
                let path = path.to_string();
                nested_hints.insert(path.clone(), "Struct".to_owned());
                last_discovered = Some(path);
            }
            Err(error) => {
                if let Some(path) = last_discovered.take()
                    && nested_hints.get(&path).is_some_and(|ty| ty == "Struct")
                {
                    nested_hints.insert(path, "Guid".to_owned());
                    continue;
                }
                return Err(error.to_string());
            }
        }
    }
    Err("nested hint discovery did not converge".to_owned())
}

/// One strict decode attempt: a standard property list holding a
/// `SaveParameter` struct.
fn decode_once(
    bytes: &[u8],
    custom_versions: &HashableIndexMap<gvas::types::Guid, u32>,
    hints: &HashMap<String, String>,
) -> Result<RawCharacter, GvasError> {
    let mut cursor = Cursor::new(bytes);
    let mut stack: Vec<String> = Vec::new();
    let mut options = gvas::properties::PropertyOptions {
        hints,
        properties_stack: &mut stack,
        custom_versions,
    };

    let mut character = RawCharacter::default();
    loop {
        let name = cursor.read_string()?;
        if name == "None" {
            break;
        }
        let value_type = cursor.read_string()?;
        let property = Property::new(&mut cursor, &value_type, true, &mut options, None)?;
        if name == "SaveParameter"
            && let Property::StructProperty(save_parameter) = &property
            && let Some(fields) = custom_fields(save_parameter)
        {
            populate(&mut character, fields);
        }
    }
    Ok(character)
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

    use crate::container::ContainerError;

    #[test]
    fn sniffing_requires_a_known_magic() {
        assert!(!crate::looks_like_sav(b"species = \"Lamball\""));
        assert!(!crate::looks_like_sav(&[0u8; 8]));
        for magic in [b"PlZ", b"PlM", b"CNK"] {
            let mut sav = vec![0u8; 16];
            sav[8..11].copy_from_slice(magic);
            assert!(crate::looks_like_sav(&sav), "{magic:?}");
        }
    }

    #[test]
    fn non_saves_are_rejected_before_parsing() {
        assert!(matches!(
            read_level_sav(b"not a save at all, definitely toml"),
            Err(SaveError::Container(ContainerError::NotASave))
        ));
    }

    #[test]
    fn corrupt_zlib_body_fails_loudly_not_by_panic() {
        let mut sav = vec![0u8; 64];
        sav[8..11].copy_from_slice(b"PlZ");
        sav[11] = 0x31;
        assert!(matches!(
            read_level_sav(&sav),
            Err(SaveError::Container(ContainerError::Zlib(_)))
        ));
    }

    #[test]
    #[ignore = "diagnostic against a real save; set PAL_SAVE_PATH"]
    fn probe_real_save_entry_shapes() {
        let path = std::env::var("PAL_SAVE_PATH").expect("set PAL_SAVE_PATH");
        let bytes = std::fs::read(path).expect("read save");
        let gvas_bytes = crate::container::decompress(&bytes).expect("container");

        let mut hints: HashMap<String, String> = SEED_HINTS
            .iter()
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect();
        let mut discovered = Vec::new();
        let file = parse_with_discovery(&gvas_bytes, &mut hints, &mut discovered).expect("gvas");

        let world = struct_fields(file.properties.get("worldSaveData")).expect("worldSaveData");
        println!("worldSaveData keys: {:?}", world.keys().collect::<Vec<_>>());
        match first(world, "CharacterSaveParameterMap") {
            Some(Property::MapProperty(MapProperty::Properties { value, .. })) => {
                println!("map entries: {}", value.len());
                if let Some((key, entry)) = value.iter().next() {
                    let debug = format!("{key:?} => {entry:?}");
                    println!(
                        "first entry (truncated): {}",
                        &debug[..debug.len().min(3000)]
                    );
                }
            }
            other => println!("unexpected map shape: {other:?}"),
        }
    }

    #[test]
    fn unknown_compression_types_are_reported() {
        let mut sav = vec![0u8; 64];
        sav[8..11].copy_from_slice(b"PlM");
        sav[11] = 0x33;
        assert!(matches!(
            read_level_sav(&sav),
            Err(SaveError::Container(ContainerError::Unsupported {
                save_type: 0x33,
                ..
            }))
        ));
    }
}
