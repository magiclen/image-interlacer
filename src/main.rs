extern crate image_interlacer;

use std::process;

use image_interlacer::*;

fn main() {
    let config = Config::from_cli();

    match config {
        Ok(config) => {
            match run(config) {
                Ok(es) => {
                    process::exit(es);
                }
                Err(error) => {
                    eprintln!("{}", error);
                    process::exit(-1);
                }
            }
        }
        Err(err) => {
            eprintln!("{}", err);
            process::exit(-1);
        }
    }
}
