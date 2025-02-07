use std::str::FromStr;

use clap::{Arg, Command};
use csv2sfz::generate_sfz;

fn main() {
    let matches = Command::new("csv2sfz-cli")
        .version("1.0.0")
        .author("momoluna")
        .about("Recursively convert any CSV file in the directory to SFZ.")
        .arg(
            Arg::new("path")
                .help("Path to the folder containing the CSV files to be converted.")
                .required(true)
                .num_args(1)
                .index(1),
        )
        .get_matches();

    
    let path = matches.get_one::<String>("path").unwrap();
    let c_path = std::ffi::CString::from_str(path).unwrap();
    let e = unsafe { generate_sfz(c_path.as_ptr()) };

    match e {
        0 => println!("Execution succeeded"),
        -1 => println!("Invalid input path"),
        -2 => println!("Error occurred while traversing directories"),
        -3 => println!("Error occurred while parsing CSV files"),
        -4 => println!("Error occurred while processing CSV expressions"),
        -5 => println!("Error occurred while saving sfz files to disk"),
        _ => println!("Unknown error"),
    }

}
