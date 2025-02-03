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
    source_file: String,
    target_file: String,
}

fn main() {
    let args: Args = Args::parse();
    if let Err(e) = run(&args) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn run(args: &Args) -> CD2ifierResult<()> {
    // Open the files containing CD1 to CD2 translation data:
    let modules_string = fs::read_to_string("src/cd2-modules.json")
        .expect("Something went wrong with the modules_map.json file.");
    let modules_map =
        json::parse(&modules_string).expect("Something went wrong with the modules_map file.");
    let pawn_stats_string = fs::read_to_string("src/pawn-stats.json")
        .expect("Something went wrong with the pawn stats data file.");
    let pawn_stats_map = json::parse(&pawn_stats_string)
        .expect("Something went wrong when parsing the pawn stats data file.");
    // Open the original difficulty file:
    let original_diff_string =
        fs::read_to_string(&args.source_file).expect("Something went wrong with the diff file.");
    let original_diff =
        json::parse(&original_diff_string).expect("Something went wrong with the diff file.");

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
    let original_resupply_cost: json::number::Number =
        if !original_diff["ResupplyCost"].is_null() && original_diff["ResupplyCost"] != 80 {
            original_diff["ResupplyCost"]
                .as_number()
                .expect("The resupply cost in the original file is not a number.")
        } else {
            80.into()
        };
    if original_diff["StartingNitra"].is_null() || original_diff["StartingNitra"] == 0 {
        target_diff["Resupply"]["Cost"] = original_resupply_cost.into();
    } else {
        target_diff["Resupply"]["Cost"] = object! {
            "Mutate": "IfFloat",
            "Value": {
              "Mutate": "ResuppliesCalled"
            },
            "==": 0,
            "Then": original_diff["StartingNitra"].clone(),
            "Else": original_resupply_cost
        }
    }

    // Loop over the original fields and translate them into the new top level modules
    // as specified in cd2-modules.json:
    for (key, val) in original_diff.entries() {
        build_top_module(&modules_map, &mut target_diff, key, val);
    }
    // Add the BaseHazard field, which is new in CD2, default to Hazard 5 for explicitness:
    build_top_module(
        &modules_map,
        &mut target_diff,
        "BaseHazard",
        &"Hazard 5".into(),
    );
    // Enemies module, copy as-is but fix the old pawn stats:
    if !original_diff["EnemyDescriptors"].is_null() {
        target_diff["EnemiesNoSync"] = original_diff["EnemyDescriptors"].clone();
        // Fix pawn stats:
        for (_, controls) in target_diff["EnemiesNoSync"].entries_mut() {
            controls.remove("UseSpawnRarityModifiers");
            if !controls["PawnStats"].is_null() {
                let pawn_stats = controls.remove("PawnStats");
                translate_pawn_stats(controls, &pawn_stats, &pawn_stats_map);
            }
        }
    }
    // Escort module, copy as-is:
    if !original_diff["EscortMule"].is_null() {
        target_diff["EscortMule"] = original_diff["EscortMule"].clone();
    }

    // Write the final string to the specified file:
    fs::write(&args.target_file, target_diff.dump()).expect("Unable to write file");

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
