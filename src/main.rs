use clap::Parser;
use json::{object, JsonValue};
use std::fs;
use std::str::FromStr;

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
                eprintln!("Field {} was missing. {}", field, msg);
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
                let mut supply_vector = Vec::new();
                for _ in 0..(starting_nitra / original_cost) as i32 {
                    supply_vector.push(0.0);
                }
                supply_vector.extend(vec![
                    original_cost - starting_nitra % original_cost,
                    original_cost,
                ]);
                supply_vector
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
                "Value": compute_supply_vector(
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
                    translate_pawn_stats(controls, &pawn_stats, &translation_data["PAWN_STATS"]);
                }
                // Remove deprecated fields:
                for (field, _) in self.original["EnemyDescriptors"][enemy].entries() {
                    if !translation_data["VALID_ENEMY_CONTROLS"].contains(field)
                        && field != "PawnStats"
                    {
                        eprintln!(
                            "Deprecated enemy control: {} in {}. Skipping.",
                            field, enemy
                        );
                        controls.remove(field);
                    }
                }
            }
        }
        DiffContainer {
            new,
            original: self.original,
        }
    }
    fn build_top_modules(self, top_modules_map: &JsonValue) -> Self {
        let mut new = self.new.clone();
        for (original_key, original_value) in self.original.entries() {
            if let Some(field_status) = top_modules_map[original_key].as_str() {
                match FieldStatus::from_str(field_status).unwrap() {
                    FieldStatus::Valid(top_module) => {
                        // This if block is trying to detect fields that have weights, since CD2 removes the
                        // "range" part of the bins:
                        if original_value.is_array()
                            && !original_value.is_empty()
                            && !original_value[0]["weight"].is_null()
                        {
                            let mut removed_ranges_arr = json::JsonValue::new_array();
                            for bin in original_value.members() {
                                removed_ranges_arr
                                    .push(object! {
                                        "weight": bin["weight"].clone(),
                                        "min": bin["range"]["min"].clone(),
                                        "max": bin["range"]["max"].clone()
                                    })
                                    .unwrap();
                            }
                            new[top_module][original_key] = removed_ranges_arr;
                        } else {
                            new[top_module][original_key] = original_value.clone();
                        }
                    }
                    FieldStatus::Deprecated => {
                        eprintln!("Deprecated field: {}. Skipping.", original_key);
                    }
                    FieldStatus::Ignored => (),
                }
            } else {
                eprintln!("Unsupported field: {}. Please open an issue.", original_key);
            }
        }
        // Here we add the BaseHazard field, defaults to HAzard 5 for explicitness:
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

    fn write_to_file(self, target_file: &str, dont_pretty_print: bool) {
        fs::write(
            target_file,
            if dont_pretty_print {
                json::stringify(self.new)
            } else {
                json::stringify_pretty(self.new, 4)
            },
        )
        .unwrap_or_else(|err| {
            panic!(
                "There was a problem when writing to the final file {}, {}",
                target_file, err
            )
        });
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
    /// Path where the translated CD2 file will be written to.
    target_file: String,
    /// If specified, the JSON will be written in compact form.
    #[arg(short, long)]
    dont_pretty_print: bool,
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
            eprintln!(
                "Unsupported pawn stat: {}. Please open an issue. Skipping.",
                stat
            );
        }
    }
}

fn parse_json(path: &str) -> JsonValue {
    let file_string = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "Something went wrong when reading the file in {}: {}",
            path, err
        )
    });
    json::parse(&file_string).unwrap_or_else(|err| {
        panic!(
            "The JSON parser couldn't parse {}: {}. Is it a proper JSON? 
            Please note that the script doesn't support multiline strings for now, as commonly found in descriptions.",
            path, err
        )
    })
}

fn run(args: &Args) {
    // Open the file containing CD1 to CD2 translation data:
    let translation_data = parse_json("src/cd2-modules.json");
    let original_file = parse_json(&args.source_file);

    DiffContainer {
        new: json::JsonValue::new_object(),
        original: &original_file,
    }
    .copy_field_if_exists("Name", "It is recommended to add a Name".into())
    .copy_field_if_exists(
        "Description",
        "It is recommended to add a Description".into(),
    )
    .build_resupply_module()
    .build_top_modules(&translation_data["TOP_MODULES"])
    .build_enemies_module(&translation_data)
    .copy_field_if_exists("EscortMule", None)
    .write_to_file(&args.target_file, args.dont_pretty_print);
}

fn main() {
    let args: Args = Args::parse();
    run(&args);
}
