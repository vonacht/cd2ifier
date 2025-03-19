use anyhow::{Context, Result};
use clap::Parser;
use itertools::{Either, Itertools};
use json::{object, JsonValue};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::{borrow::Cow, io::IsTerminal};
use tracing::{event, Level};

struct DiffContainer<'a> {
    new: JsonValue,
    original: &'a JsonValue,
}

impl<'a> DiffContainer<'a> {
    fn copy_field_if_exists(self, field: &str, err_msg: Option<&str>) -> Self {
        if self.original.has_key(field) {
            let mut new = self.new.clone();
            new[field] = self.original[field].clone();
            DiffContainer {
                new,
                original: self.original,
            }
        } else {
            if let Some(msg) = err_msg {
                event!(Level::WARN, "Field [{field}] was missing. [{msg}]");
            }
            self
        }
    }
    fn build_resupply_module(self) -> Self {
        // Resupply module. Copy the cost if StartingNitra is 0 or missing, otherwise add
        // the corresponding nitra mutator

        fn compute_supply_vector(starting_nitra: f64, original_cost: f64) -> Vec<f64> {
            if starting_nitra <= original_cost {
                vec![original_cost - starting_nitra, original_cost]
            } else {
                std::iter::repeat(0.0)
                    .take((starting_nitra / original_cost) as usize)
                    .chain(vec![
                        original_cost - starting_nitra % original_cost,
                        original_cost,
                    ])
                    .collect()
            }
        }

        let mut new = self.new.clone();
        let original_resupply_cost: f64 =
            if !self.original["ResupplyCost"].is_null() && self.original["ResupplyCost"] != 80 {
                self.original["ResupplyCost"].as_f64().unwrap()
            } else {
                80.00
            };
        if self.original["StartingNitra"].is_null() || self.original["StartingNitra"] == 0 {
            new["Resupply"]["Cost"] = original_resupply_cost.into();
        } else {
            new["Resupply"]["Cost"] = object! {
                "Mutate": "ByResuppliesCalled",
                "Values": compute_supply_vector(
                    self.original["StartingNitra"].as_f64().unwrap(),
                    original_resupply_cost
                )
            }
        }
        DiffContainer {
            new,
            original: self.original,
        }
    }
    fn build_enemies_module(self, translation_data: &JsonValue) -> Self {
        // Enemies module, copy as-is but fix the old pawn stats and remove deprecated fields:
        let mut new = self.new.clone();
        if !self.original["EnemyDescriptors"].is_null() {
            new["EnemiesNoSync"] = self.original["EnemyDescriptors"].clone();
            // Fix pawn stats:
            for (enemy, controls) in new["EnemiesNoSync"].entries_mut() {
                if !controls["PawnStats"].is_null() {
                    let pawn_stats = controls.remove("PawnStats");
                    translate_pawn_stats(
                        controls,
                        &pawn_stats,
                        &translation_data["PAWN_STATS"],
                        enemy,
                    );
                }
                // Remove deprecated fields:
                for (field, _) in self.original["EnemyDescriptors"][enemy].entries() {
                    if !translation_data["VALID_ENEMY_CONTROLS"].contains(field)
                        && field != "PawnStats"
                    {
                        event!(
                            Level::INFO,
                            "Deprecated or mistyped enemy control: [{field}] in [{enemy}]. Skipping."
                        );
                        controls.remove(field);
                    }
                }
                // Elite detection;
                if controls.has_key("Elite")
                    && controls["Elite"] == true
                    && !(translation_data["VANILLA_ELITE_ENEMIES"])
                        .contains(controls["Base"].clone())
                    && (translation_data["VANILLA_ELITE_ENEMIES"]).contains(enemy)
                {
                    event!(
                        Level::INFO,
                        "Non-vanilla elite enemy detected with base: [{}]",
                        controls["Base"].clone()
                    );
                    controls["ForceEliteBase"] = enemy.into();
                }
            }
        }
        DiffContainer {
            new,
            original: self.original,
        }
    }
    fn build_top_modules(self, top_modules_map: &JsonValue) -> Self {
        fn update_if_range_array(original_value: &JsonValue) -> JsonValue {
            // This if block is trying to detect fields that have weights, since CD2 removes the
            // "range" part of the bins:
            if original_value.is_array()
                && !original_value.is_empty()
                && !original_value[0]["weight"].is_null()
            {
                original_value
                    .members()
                    .map(|arr| {
                        object! {
                            "weight": arr["weight"].clone(),
                            "min": arr["range"]["min"].clone(),
                            "max": arr["range"]["max"].clone()
                        }
                    })
                    .collect::<Vec<JsonValue>>()
                    .into()
            } else {
                original_value.clone()
            }
        }

        let mut new = self.new.clone();
        for (original_key, original_value) in self.original.entries() {
            if let Some(field_status) = top_modules_map[original_key].as_str() {
                match FieldStatus::from_str(field_status).unwrap() {
                    FieldStatus::Valid(top_module) => {
                        new[top_module][original_key] = update_if_range_array(original_value);
                    }
                    FieldStatus::Deprecated => {
                        event!(Level::INFO, "Deprecated field: [{original_key}]. Skipping.");
                    }
                    FieldStatus::Ignored => (),
                }
            } else {
                event!(
                    Level::WARN,
                    "Unsupported field: [{original_key}]. Please open an issue."
                );
            }
        }
        // Here we add the BaseHazard field, defaults to Hazard 5 for explicitness:
        new["DifficultySetting"]["BaseHazard"] = "Hazard 5".into();
        // Change the name of StationaryEnemies, which in CD2 changed name to StationaryPool:
        let stationary_enemies = new["Pools"].remove("StationaryEnemies");
        if !stationary_enemies.is_null() {
            new["Pools"]["StationaryPool"] = stationary_enemies
        }
        DiffContainer {
            new,
            original: self.original,
        }
    }

    fn write_to_file(
        self,
        target_file: &str,
        dont_pretty_print: bool,
        multilines: Option<String>,
    ) -> Result<()> {
        let append_multilines = |mlines| -> JsonValue {
            let mut with_multilines = self.new.clone();
            with_multilines["Description"] =
                format! {"{}{}", self.new["Description"].as_str().unwrap(), mlines}.into();
            with_multilines
        };

        fs::write(
            target_file,
            if dont_pretty_print {
                if let Some(mlines) = multilines {
                    json::stringify(append_multilines(mlines))
                } else {
                    json::stringify(self.new)
                }
            } else if let Some(mlines) = multilines {
                recover_multilines(&json::stringify_pretty(self.new, 4), &mlines)
            } else {
                json::stringify_pretty(self.new, 4)
            },
        )
        .with_context(|| {
            format!(
                "There was a problem when writing to the final file {}",
                target_file
            )
        })
    }
}

enum FieldStatus {
    Deprecated,
    Ignored,
    Valid(String),
}

impl FromStr for FieldStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<FieldStatus, Self::Err> {
        match input {
            "deprecated" => Ok(FieldStatus::Deprecated),
            "ignore" => Ok(FieldStatus::Ignored),
            _ => Ok(FieldStatus::Valid(input.to_string())),
        }
    }
}

#[derive(Parser, Debug)]
struct Args {
    /// Path to the CD1 file to be converted.
    source_file: String,
    /// Path where the translated CD2 file will be written to. If not specified, the script will
    /// append .cd2 to the original file name
    target_file: Option<String>,
    /// If specified, the JSON will be written in compact form.
    #[arg(short, long)]
    dont_pretty_print: bool,
}

fn translate_pawn_stats(
    controls: &mut JsonValue,
    pawn_stats: &JsonValue,
    pawn_stats_map: &JsonValue,
    enemy: &str,
) {
    for (stat, value) in pawn_stats.entries() {
        if !pawn_stats_map[stat].is_null() {
            let new_module = pawn_stats_map[stat]["CD2_module"].as_str().unwrap();
            let new_field = pawn_stats_map[stat]["CD2_field"].as_str().unwrap();
            let new_value = if new_module != "Resistances" || stat == "PST_DamageResistance" {
                value
            } else {
                &(1.0 - value.as_f64().unwrap()).into()
            };
            if new_module == "None" {
                controls[new_field] = new_value.clone();
            } else {
                controls[new_module][new_field] = new_value.clone();
            }
        } else {
            event!(
                Level::WARN,
                "Unsupported pawn stat: [{stat}] on enemy [{enemy}]. Please open an issue. Skipping."
            );
        }
    }
}

fn file_to_string(path: &str) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("Something went wrong when reading the file {}", path))
}

fn parse_json(file_str: &str) -> Result<JsonValue> {
    json::parse(file_str)
        .with_context(|| "The JSON parser couldn't parse the file. Is it a proper JSON?")
}

fn parse_json_with_multilines(file_path: &str) -> Result<(JsonValue, Option<String>)> {
    let original_file_str = file_to_string(file_path)?;
    let (original_file_str, multilines) = maybe_extract_multilines(&original_file_str);
    Ok((parse_json(&original_file_str)?, multilines))
}
/// This function checks for files that have multiline descriptions.
/// It returns either the original file (if no multilines) or the
/// original file with multilines removed, plus the multiline Strings as an Option
fn maybe_extract_multilines(file_str: &str) -> (Cow<str>, Option<String>) {
    let mut multiline_idx = (-1, -1);
    for (line_num, line) in file_str.lines().enumerate() {
        if multiline_idx.0 == -1 {
            if line.trim().starts_with("\"Description\"") {
                if line.trim_end().ends_with("\",") {
                    // This file contains no multilines
                    break;
                } else {
                    multiline_idx.0 = (line_num + 1) as isize;
                }
            }
        } else if line.trim().starts_with("\"") && line.trim() != "\"," {
            multiline_idx.1 = (line_num - 1) as isize;
            break;
        }
    }
    if multiline_idx.0 == -1 {
        (Cow::Borrowed(file_str), None)
    } else {
        event!(Level::INFO, "Multiline description detected. Saving.");
        let (multilines_removed, multilines): (Vec<_>, Vec<_>) =
            file_str.lines().enumerate().partition_map(|(ii, line)| {
                if ii == (multiline_idx.0 - 1) as usize {
                    Either::Left(format!("{}{}", line, "\","))
                } else if ii >= multiline_idx.0 as usize && ii <= multiline_idx.1 as usize {
                    Either::Right(line)
                } else {
                    Either::Left(line.to_string())
                }
            });
        (
            Cow::Owned(multilines_removed.join("\n")),
            Some(multilines.join("\n")),
        )
    }
}
fn recover_multilines(json_string: &str, multilines: &str) -> String {
    event!(Level::INFO, "Recovering multiline description.");
    let mut recovered_file = Vec::new();
    let mut description_found = false;
    for line in json_string.lines() {
        if !description_found {
            if line.trim().starts_with("\"Description\"") {
                description_found = true;
                recovered_file.push(line.trim_end().trim_end_matches("\","));
            } else {
                recovered_file.push(line);
            }
        } else {
            recovered_file.push(multilines);
            description_found = false;
            recovered_file.push(line);
        }
    }
    recovered_file.join("\n")
}

fn file_name<'a>(source: &'a str, target: Option<&'a str>) -> Cow<'a, str> {
    if let Some(name) = target {
        Cow::Borrowed(name)
    } else {
        let file_name = Path::new(source).file_stem().unwrap().to_str().unwrap();
        let extension = Path::new(source).extension();
        Cow::Owned(if let Some(extension) = extension {
            format!("{}.cd2.{}", file_name, extension.to_str().unwrap())
        } else {
            format!("{file_name}.cd2")
        })
    }
}

fn run(args: &Args) -> Result<()> {
    // Open the file containing CD1 to CD2 translation data:
    let translation_data = parse_json(&file_to_string("src/cd2-modules.json")?)?;
    let (cd1_json, multilines) = parse_json_with_multilines(&args.source_file)?;

    DiffContainer {
        new: json::JsonValue::new_object(),
        original: &cd1_json,
    }
    .copy_field_if_exists("Name", "It is recommended to add a Name.".into())
    .copy_field_if_exists(
        "Description",
        "It is recommended to add a Description.".into(),
    )
    .build_resupply_module()
    .build_top_modules(&translation_data["TOP_MODULES"])
    .build_enemies_module(&translation_data)
    .copy_field_if_exists("EscortMule", None)
    .write_to_file(
        &file_name(&args.source_file, args.target_file.as_deref()),
        args.dont_pretty_print,
        multilines,
    )?;

    Ok(())
}

fn main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(std::io::stdout().is_terminal())
        .init();
    let args: Args = Args::parse();
    if let Err(e) = run(&args) {
        event!(Level::ERROR, "{:#}", e);
        event!(Level::ERROR, "Conversion unfinished. Exiting.");
    }
}
