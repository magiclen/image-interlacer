//! # Image Interlacer
//! It helps you interlace an image or multiple images for web-page usage.

extern crate clap;
extern crate image_convert;
extern crate path_absolutize;
extern crate scanner_rust;
extern crate starts_ends_with_caseless;
extern crate walkdir;
extern crate num_cpus;
extern crate threadpool;
extern crate pathdiff;

use std::env;
use std::path::Path;
use std::fs;
use std::sync::{Mutex, Arc};
use std::io::{self, Write};

use clap::{App, Arg};

use path_absolutize::*;

use image_convert::{InterlaceType, ImageResource, ImageIdentify, identify, magick_rust::bindings};

use scanner_rust::Scanner;

use starts_ends_with_caseless::{StartsWithCaseless, StartsWithCaselessMultiple};

use walkdir::WalkDir;

use threadpool::ThreadPool;

// TODO -----Config START-----

const APP_NAME: &str = "Image Interlacer";
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

#[derive(Debug)]
pub struct Config {
    pub input: String,
    pub output: Option<String>,
    pub single_thread: bool,
    pub force: bool,
    pub allow_gif: bool,
    pub remain_profile: bool,
}

impl Config {
    pub fn from_cli() -> Result<Config, String> {
        let arg0 = env::args().next().unwrap();
        let arg0 = Path::new(&arg0).file_stem().unwrap().to_str().unwrap();

        let examples = vec![
            "/path/to/image                           # Check /path/to/image and make it interlaced",
            "/path/to/folder                          # Check /path/to/folder and make images inside it interlaced",
            "/path/to/image  -o /path/to/image2       # Check /path/to/image and make it interlaced, and save it to /path/to/image2",
            "/path/to/folder -o /path/to/folder2      # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2",
            "/path/to/folder -o /path/to/folder2 -f   # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2 without overwriting checks",
            "/path/to/folder --allow-gif -r           # Check /path/to/folder and make images inside it including GIF images interlaced and also remain their profiles",
        ];

        let matches = App::new(APP_NAME)
            .version(CARGO_PKG_VERSION)
            .author(CARGO_PKG_AUTHORS)
            .about(format!("It helps you interlace an image or multiple images for web-page usage.\n\nEXAMPLES:\n{}", examples.iter()
                .map(|e| format!("  {} {}\n", arg0, e))
                .collect::<Vec<String>>()
                .concat()
            ).as_str()
            )
            .arg(Arg::with_name("INPUT_PATH")
                .required(true)
                .help("Assigns an image or a directory for image interlacing. It should be a path of a file or a directory")
                .takes_value(true)
            )
            .arg(Arg::with_name("OUTPUT_PATH")
                .required(false)
                .long("output")
                .short("o")
                .help("Assigns a destination of your generated files. It should be a path of a directory or a file depending on your input path")
                .takes_value(true)
            )
            .arg(Arg::with_name("SINGLE_THREAD")
                .long("single-thread")
                .short("s")
                .help("Uses only one thread")
            )
            .arg(Arg::with_name("FORCE")
                .long("force")
                .short("f")
                .help("Forces to overwrite files")
            )
            .arg(Arg::with_name("ALLOW_GIF")
                .long("allow-gif")
                .help("Allows to do GIF interlacing")
            )
            .arg(Arg::with_name("REMAIN_PROFILE")
                .long("remain-profile")
                .short("r")
                .help("Remains the profiles of all images")
            )
            .after_help("Enjoy it! https://magiclen.org")
            .get_matches();

        let input = matches.value_of("INPUT_PATH").unwrap().to_string();

        let output = matches.value_of("OUTPUT_PATH").map(|s| s.to_string());

        let single_thread = matches.is_present("SINGLE_THREAD");

        let force = matches.is_present("FORCE");

        let allow_gif = matches.is_present("ALLOW_GIF");

        let remain_profile = matches.is_present("REMAIN_PROFILE");

        Ok(Config {
            input,
            output,
            single_thread,
            force,
            allow_gif,
            remain_profile,
        })
    }
}

// TODO -----Config END-----

pub fn run(config: Config) -> Result<i32, String> {
    let (input_path, is_file) = match Path::new(config.input.as_str()).canonicalize() {
        Ok(path) => {
            let metadata = path.metadata().map_err(|err| err.to_string())?;

            let file_type = metadata.file_type();

            let is_file = file_type.is_file();

            if !path.is_dir() && !is_file {
                return Err(format!("`{}` is not an existing file or a directory.", path.to_string_lossy()));
            }

            (path, is_file)
        }
        Err(err) => {
            return Err(err.to_string());
        }
    };

    let output_path = match config.output.as_ref() {
        Some(output) => {
            let output_path = Path::new(output).absolutize().map_err(|err| err.to_string())?;

            if let Ok(metadata) = output_path.metadata() {
                let file_type = metadata.file_type();

                if file_type.is_file() {
                    if !is_file {
                        return Err(format!("`{}` is not a file.", output_path.to_string_lossy()));
                    }
                } else if file_type.is_dir() {
                    if is_file {
                        return Err(format!("`{}` is not a directory.", output_path.to_string_lossy()));
                    }
                }
            }

            Some(output_path)
        }
        None => None
    };

    let sc: Arc<Mutex<Scanner<io::Stdin>>> = Arc::new(Mutex::new(Scanner::scan_stream(io::stdin())));
    let overwriting: Arc<Mutex<u8>> = Arc::new(Mutex::new(0));

    if is_file {
        interlacing(config.allow_gif, config.remain_profile, config.force, &sc, &overwriting, &input_path, output_path.as_ref().map(|p| p.as_path()))?;
    } else {
        let mut image_paths = Vec::new();

        for entry in WalkDir::new(&input_path).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();

            if !p.is_file() {
                continue;
            }

            if let Some(extension) = p.extension() {
                if let Some(extension) = extension.to_str() {
                    let mut allow_extensions = vec!["jpg", "jpeg", "png"];

                    if config.allow_gif {
                        allow_extensions.push("gif");
                    }

                    if extension.starts_with_caseless_ascii_multiple(&allow_extensions) {
                        image_paths.push(p.canonicalize().unwrap());
                    }
                }
            }
        }

        if config.single_thread {
            for image_path in image_paths {
                let output_path = match output_path.as_ref() {
                    Some(output_path) => {
                        let p = pathdiff::diff_paths(&image_path, &input_path).unwrap();

                        let output_path = output_path.join(&p);

                        Some(output_path)
                    }
                    None => None
                };

                if let Err(err) = interlacing(config.allow_gif, config.remain_profile, config.force, &sc, &overwriting, image_path.as_path(), output_path.as_ref().map(|p| p.as_path())) {
                    eprintln!("{}", err);
                    io::stderr().flush().map_err(|err| err.to_string())?;
                }
            }
        } else {
            let cpus = num_cpus::get();

            let pool = ThreadPool::new(cpus * 2);

            for image_path in image_paths {
                let allow_gif = config.allow_gif;
                let remain_profile = config.remain_profile;
                let force = config.force;
                let output_path = match output_path.as_ref() {
                    Some(output_path) => {
                        let p = pathdiff::diff_paths(&image_path, &input_path).unwrap();

                        let output_path = output_path.join(&p);

                        Some(output_path)
                    }
                    None => None
                };

                let sc = sc.clone();
                let overwriting = overwriting.clone();

                pool.execute(move || {
                    if let Err(err) = interlacing(allow_gif, remain_profile, force, &sc, &overwriting, image_path.as_path(), output_path.as_ref().map(|p| p.as_path())) {
                        eprintln!("{}", err);
                        io::stderr().flush().unwrap();
                    }
                });
            }

            pool.join();
        }
    }

    Ok(0)
}

fn interlacing(allow_gif: bool, remain_profile: bool, force: bool, sc: &Arc<Mutex<Scanner<io::Stdin>>>, overwriting: &Arc<Mutex<u8>>, input_path: &Path, output_path: Option<&Path>) -> Result<(), String> {
    let mut output = None;

    let input_image_resource = ImageResource::from_path(&input_path);

    let input_identify: ImageIdentify = identify(&mut output, &input_image_resource).map_err(|err| err.to_string())?;

    match input_identify.interlace {
        InterlaceType::NoInterlace | InterlaceType::UndefinedInterlace => {
            let allow_interlacing = match input_identify.format.as_str() {
                "JPEG" | "PNG" => true,
                "GIF" => allow_gif,
                _ => false
            };

            if allow_interlacing {
                let mut output = Some(None);

                let input_identify = identify(&mut output, &input_image_resource).map_err(|err| err.to_string())?;

                if let Some(magic_wand) = output {
                    let mut magic_wand = magic_wand.unwrap();

                    magic_wand.set_interlace_scheme(InterlaceType::LineInterlace.ordinal() as bindings::InterlaceType).map_err(|err| err.to_string())?;

                    if !remain_profile {
                        magic_wand.profile_image("*", None)?;
                    }

                    let output_path = match output_path {
                        Some(output_path) => {
                            if output_path.exists() {
                                if !force {
                                    let mutex_guard = overwriting.lock().unwrap();

                                    let output_path_string = output_path.to_string_lossy();

                                    let allow_overwrite = loop {
                                        print!("`{}` exists, do you want to overwrite it? [y/n] ", output_path_string);
                                        io::stdout().flush().map_err(|_| "Cannot flush stdout.".to_string())?;

                                        let token = sc.lock().unwrap().next().map_err(|_| "Cannot read from stdin.".to_string())?.ok_or("Read EOF.".to_string())?;

                                        if token.starts_with_caseless_ascii("y") {
                                            break true;
                                        } else if token.starts_with_caseless_ascii("n") {
                                            break false;
                                        }
                                    };

                                    drop(mutex_guard);

                                    if !allow_overwrite {
                                        return Ok(());
                                    }
                                }
                            } else {
                                fs::create_dir_all(output_path.parent().unwrap()).map_err(|err| err.to_string())?;
                            }

                            output_path
                        }
                        None => input_path
                    };
                    let temp = magic_wand.write_image_blob(input_identify.format.as_str())?;

                    fs::write(&output_path, temp).map_err(|err| err.to_string())?;

                    let mutex_guard = overwriting.lock().unwrap();

                    println!("`{}` has been interlaced.", output_path.to_string_lossy());
                    io::stdout().flush().map_err(|_| "Cannot flush stdout.".to_string())?;

                    drop(mutex_guard);
                } else {
                    unreachable!();
                }
            }
        }
        _ => {}
    }

    Ok(())
}