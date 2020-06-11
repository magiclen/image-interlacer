#[macro_use]
extern crate concat_with;
extern crate clap;
extern crate terminal_size;

extern crate path_absolutize;
extern crate starts_ends_with_caseless;

extern crate scanner_rust;

extern crate num_cpus;
extern crate threadpool;

extern crate image_convert;

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use clap::{App, Arg};
use terminal_size::terminal_size;

use path_absolutize::Absolutize;
use starts_ends_with_caseless::{StartsWithCaseless, StartsWithCaselessMultiple};

use scanner_rust::generic_array::typenum::U8;
use scanner_rust::Scanner;

use threadpool::ThreadPool;

const APP_NAME: &str = "Image Interlacer";
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

fn main() -> Result<(), String> {
    let matches = App::new(APP_NAME)
        .set_term_width(terminal_size().map(|(width, _)| width.0 as usize).unwrap_or(0))
        .version(CARGO_PKG_VERSION)
        .author(CARGO_PKG_AUTHORS)
        .about(concat!("It helps you generate favicons with different formats and sizes.\n\nEXAMPLES:\n", concat_line!(prefix "image-interlacer ",
                "/path/to/image                           # Check /path/to/image and make it interlaced",
                "/path/to/folder                          # Check /path/to/folder and make images inside it interlaced",
                "/path/to/image  -o /path/to/image2       # Check /path/to/image and make it interlaced, and save it to /path/to/image2",
                "/path/to/folder -o /path/to/folder2      # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2",
                "/path/to/folder -o /path/to/folder2 -f   # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2 without overwriting checks",
                "/path/to/folder --allow-gif -r           # Check /path/to/folder and make images inside it including GIF images interlaced and also remain their profiles",
            )))
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

    let input = matches.value_of("INPUT_PATH").unwrap();
    let output = matches.value_of("OUTPUT_PATH");

    let single_thread = matches.is_present("SINGLE_THREAD");
    let force = matches.is_present("FORCE");
    let allow_gif = matches.is_present("ALLOW_GIF");
    let remain_profile = matches.is_present("REMAIN_PROFILE");

    let input_path = Path::new(input);

    let is_dir = input_path.metadata().map_err(|err| err.to_string())?.is_dir();

    let output_path = match output {
        Some(output) => {
            let output_path = Path::new(output);

            if is_dir {
                match output_path.metadata() {
                    Ok(metadata) => {
                        if metadata.is_dir() {
                            Some(output_path)
                        } else {
                            return Err(format!(
                                "`{}` is not a directory.",
                                output_path
                                    .absolutize()
                                    .map_err(|err| err.to_string())?
                                    .to_string_lossy()
                            ));
                        }
                    }
                    Err(_) => {
                        fs::create_dir_all(output_path).map_err(|err| err.to_string())?;

                        Some(output_path)
                    }
                }
            } else if output_path.is_dir() {
                return Err(format!(
                    "`{}` is not a file.",
                    output_path.absolutize().map_err(|err| err.to_string())?.to_string_lossy()
                ));
            } else {
                Some(output_path)
            }
        }
        None => None,
    };

    let sc: Arc<Mutex<Scanner<io::Stdin, U8>>> =
        Arc::new(Mutex::new(Scanner::new2(io::stdin())));
    let overwriting: Arc<Mutex<u8>> = Arc::new(Mutex::new(0));

    if is_dir {
        let mut image_paths = Vec::new();

        for dir_entry in input_path.read_dir().map_err(|err| err.to_string())? {
            let dir_entry = dir_entry.map_err(|err| err.to_string())?;

            if !dir_entry.metadata().map_err(|err| err.to_string())?.is_file() {
                continue;
            }

            let p = dir_entry.path();

            if let Some(extension) = p.extension() {
                if let Some(extension) = extension.to_str() {
                    let mut allow_extensions = vec!["jpg", "jpeg", "png"];

                    if allow_gif {
                        allow_extensions.push("gif");
                    }

                    if extension.starts_with_caseless_ascii_multiple(&allow_extensions).is_some() {
                        image_paths.push(p);
                    }
                }
            }
        }

        if single_thread {
            for image_path in image_paths {
                let output_path = match output_path.as_ref() {
                    Some(output_path) => {
                        let p = pathdiff::diff_paths(&image_path, &input_path).unwrap();

                        let output_path = output_path.join(&p);

                        Some(output_path)
                    }
                    None => None,
                };

                if let Err(err) = interlacing(
                    allow_gif,
                    remain_profile,
                    force,
                    &sc,
                    &overwriting,
                    image_path.as_path(),
                    output_path.as_deref(),
                ) {
                    eprintln!("{}", err);
                    io::stderr().flush().map_err(|err| err.to_string())?;
                }
            }
        } else {
            let cpus = num_cpus::get();

            let pool = ThreadPool::new(cpus * 2);

            for image_path in image_paths {
                let sc = sc.clone();
                let overwriting = overwriting.clone();
                let output_path = match output_path.as_ref() {
                    Some(output_path) => {
                        let p = pathdiff::diff_paths(&image_path, &input_path).unwrap();

                        let output_path = output_path.join(&p);

                        Some(output_path)
                    }
                    None => None,
                };

                pool.execute(move || {
                    if let Err(err) = interlacing(
                        allow_gif,
                        remain_profile,
                        force,
                        &sc,
                        &overwriting,
                        image_path.as_path(),
                        output_path.as_deref(),
                    ) {
                        eprintln!("{}", err);
                        io::stderr().flush().unwrap();
                    }
                });
            }

            pool.join();
        }
    } else {
        interlacing(allow_gif, remain_profile, force, &sc, &overwriting, &input_path, output_path)?;
    }

    Ok(())
}

fn interlacing(
    allow_gif: bool,
    remain_profile: bool,
    force: bool,
    sc: &Arc<Mutex<Scanner<io::Stdin, U8>>>,
    overwriting: &Arc<Mutex<u8>>,
    input_path: &Path,
    output_path: Option<&Path>,
) -> Result<(), String> {
    let mut output = None;

    let input_image_resource = image_convert::ImageResource::from_path(&input_path);

    let input_identify = image_convert::identify(&mut output, &input_image_resource)
        .map_err(|err| err.to_string())?;

    match input_identify.interlace {
        image_convert::InterlaceType::NoInterlace
        | image_convert::InterlaceType::UndefinedInterlace => {
            let allow_interlacing = match input_identify.format.as_str() {
                "JPEG" | "PNG" => true,
                "GIF" => allow_gif,
                _ => false,
            };

            if allow_interlacing {
                let mut output = Some(None);

                let input_identify = image_convert::identify(&mut output, &input_image_resource)
                    .map_err(|err| err.to_string())?;

                match output {
                    Some(magic_wand) => {
                        let mut magic_wand = magic_wand.unwrap();

                        magic_wand
                            .set_interlace_scheme(
                                image_convert::InterlaceType::LineInterlace.ordinal()
                                    as image_convert::magick_rust::bindings::InterlaceType,
                            )
                            .map_err(|err| err.to_string())?;

                        if !remain_profile {
                            magic_wand.profile_image("*", None)?;
                        }

                        let output_path = match output_path.as_ref() {
                            Some(output_path) => {
                                if output_path.exists() {
                                    if !force {
                                        let mutex_guard = overwriting.lock().unwrap();

                                        let output_path_string = output_path.to_string_lossy();

                                        let allow_overwrite = loop {
                                            print!(
                                                "`{}` exists, do you want to overwrite it? [y/n] ",
                                                output_path_string
                                            );
                                            io::stdout()
                                                .flush()
                                                .map_err(|_| "Cannot flush stdout.".to_string())?;

                                            let token = sc
                                                .lock()
                                                .unwrap()
                                                .next()
                                                .map_err(|_| "Cannot read from stdin.".to_string())?
                                                .ok_or_else(|| "Read EOF.".to_string())?;

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
                                    fs::create_dir_all(output_path.parent().unwrap())
                                        .map_err(|err| err.to_string())?;
                                }

                                output_path
                            }
                            None => input_path,
                        };
                        let temp = magic_wand.write_image_blob(input_identify.format.as_str())?;

                        fs::write(&output_path, temp).map_err(|err| err.to_string())?;

                        let mutex_guard = overwriting.lock().unwrap();

                        println!("`{}` has been interlaced.", output_path.to_string_lossy());
                        io::stdout().flush().map_err(|_| "Cannot flush stdout.".to_string())?;

                        drop(mutex_guard);
                    }
                    None => unreachable!(),
                }
            }
        }
        _ => (),
    }

    Ok(())
}
