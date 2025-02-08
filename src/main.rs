use clap::Parser;
use json::{object, JsonValue};
use std::error::Error;
use std::fs;
use std::str::FromStr;

type CD2ifierResult<T> = Result<T, Box<dyn Error>>;

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
    /// Path where the translated CD2 file will be written to.
    target_file: String,
    /// If specified, the JSON will be formatted in compact form.
    #[arg(short, long)]
    dont_pretty_print: bool,
}

fn main() {
    let args: Args = Args::parse();
    if let Err(e) = run(&args) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn open(path: &str) -> JsonValue {
    let file_string = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "Something went wrong when reading the file in {}: {}",
            path, err
        )
    });
    json::parse(&file_string).unwrap_or_else(|err| {
        panic!(
            "The JSON parser couldn't parse {}: {}. Is it a proper JSON?",
            path, err
        )
    })
}

fn run(args: &Args) -> CD2ifierResult<()> {
    // Open the files containing CD1 to CD2 translation data:
    let translation_data = open("src/cd2-modules.json");
    // Open the original difficulty file:
    let original_diff = open(&args.source_file);

    let mut target_diff = json::JsonValue::new_object();

    // Name and description, copy as-is:
    if !original_diff["Description"].is_null() {
        target_diff["Description"] = original_diff["Description"].clone();
    } else {
        println!("The original file doesn't have a description, skipping.")
    }
    if !original_diff["Name"].is_null() {
        target_diff["Name"] = original_diff["Name"].clone();
    } else {
        println!("The original file doesn't have a name, skipping. It is recommended to add one.")
    }

    // Resupply module. Copy the cost if StartingNitra is 0 or missing, otherwise add
    // the corresponding nitra mutator:
    let original_resupply_cost: f64 =
        if !original_diff["ResupplyCost"].is_null() && original_diff["ResupplyCost"] != 80 {
            original_diff["ResupplyCost"].as_f64().unwrap()
        } else {
            80.00
        };
    if original_diff["StartingNitra"].is_null() || original_diff["StartingNitra"] == 0 {
        target_diff["Resupply"]["Cost"] = original_resupply_cost.into();
    } else {
        let first_resupply: f64 = if original_diff["StartingNitra"].as_f64().unwrap()
            >= original_resupply_cost
        {
            println!("Warning: This script does not support a StartingNitra higher than the resupply Cost for now.");
            println!("It will set up the first supply free.");
            0.0
        } else {
            original_resupply_cost - original_diff["StartingNitra"].as_f64().unwrap()
        };

        target_diff["Resupply"]["Cost"] = object! {
            "Mutate": "IfFloat",
            "Value": {
              "Mutate": "ResuppliesCalled"
            },
            "==": 0,
            "Then": first_resupply,
            "Else": original_resupply_cost
        }
    }

    // Loop over the original fields and translate them into the new top level modules
    // as specified in cd2-modules.json:
    for (key, val) in original_diff.entries() {
        build_top_module(&translation_data["TOP_MODULES"], &mut target_diff, key, val);
    }
    // Add the BaseHazard field, which is new in CD2, default to Hazard 5 for explicitness:
    target_diff["DifficultySetting"]["BaseHazard"] = "Hazard 5".into();
    // Change the name of StationaryEnemies, which is StationaryPool in CD2:
    let stationary_enemies = target_diff["Pools"].remove("StationaryEnemies");
    if !stationary_enemies.is_null() {
        target_diff["Pools"]["StationaryPool"] = stationary_enemies
    }
    // Enemies module, copy as-is but fix the old pawn stats and remove deprecated fields:
    if !original_diff["EnemyDescriptors"].is_null() {
        target_diff["EnemiesNoSync"] = original_diff["EnemyDescriptors"].clone();
        // Fix pawn stats:
        for (enemy, controls) in target_diff["EnemiesNoSync"].entries_mut() {
            if !controls["PawnStats"].is_null() {
                let pawn_stats = controls.remove("PawnStats");
                translate_pawn_stats(controls, &pawn_stats, &translation_data["PAWN_STATS"]);
            }
            // Remove deprecated fields:
            for (field, _) in original_diff["EnemyDescriptors"][enemy].entries() {
                if !translation_data["VALID_ENEMY_CONTROLS"].contains(field) && field != "PawnStats"
                {
                    println!(
                        "Deprecated enemy control: {} in {}. Skipping.",
                        field, enemy
                    );
                    controls.remove(field);
                }
            }
        }
    }
    // Escort module, copy as-is:
    if !original_diff["EscortMule"].is_null() {
        target_diff["EscortMule"] = original_diff["EscortMule"].clone();
    }

    let write_func = if args.dont_pretty_print {
        json::stringify(target_diff)
    } else {
        json::stringify_pretty(target_diff, 4)
    };

    fs::write(&args.target_file, write_func).unwrap_or_else(|err| {
        panic!(
            "There was a problem when writing to the final file {}, {}",
            &args.target_file, err
        )
    });

    Ok(())
}

fn build_top_module(
    module_map: &JsonValue,
    new_file: &mut JsonValue,
    original_key: &str,
    new_value: &JsonValue,
) {
    if let Some(field_status) = module_map[original_key].as_str() {
        match FieldStatus::from_str(field_status).unwrap() {
            FieldStatus::Valid(top_module) => {
                // This if block is trying to detect fields that have weights, since CD2 removes the
                // "range" part of the bins:
                if new_value.is_array()
                    && !new_value.is_empty()
                    && !new_value[0]["weight"].is_null()
                {
                    let mut removed_ranges_arr = json::JsonValue::new_array();
                    for bin in new_value.members() {
                        removed_ranges_arr
                            .push(object! {
                                "weight": bin["weight"].clone(),
                                "min": bin["range"]["min"].clone(),
                                "max": bin["range"]["max"].clone()
                            })
                            .unwrap();
                    }
                    new_file[top_module][original_key] = removed_ranges_arr;
                } else {
                    new_file[top_module][original_key] = new_value.clone();
                }
            }
            FieldStatus::Deprecated => {
                println!("Deprecated field: {}. Skipping.", original_key);
            }
            FieldStatus::Ignored => (),
        }
    } else {
        println!("Unsupported field: {}. Please open an issue.", original_key);
    }
}

fn translate_pawn_stats(
    controls: &mut JsonValue,
    pawn_stats: &JsonValue,
    pawn_stats_map: &JsonValue,
) {
    for (stat, value) in pawn_stats.entries() {
        if !pawn_stats_map[stat].is_null() {
            let new_module = pawn_stats_map[stat]["CD2_module"].as_str().unwrap();
            let new_field = pawn_stats_map[stat]["CD2_field"].as_str().unwrap();
            let new_value = if stat == "PST_DamageResistance" || stat == "PST_MovementSpeed" {
                value
            } else {
                &(1.0 - value.as_f64().unwrap()).into()
            };
            controls[new_module][new_field] = new_value.clone();
        } else {
            println!("Unsupported pawn stat: {}. Please open an issue.", stat);
        }
    }
}
