mod cli;

use std::{
    fs, io,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context};
use cli::*;
use scanner_rust::{generic_array::typenum::U8, Scanner};
use str_utils::EqIgnoreAsciiCaseMultiple;
use threadpool::ThreadPool;
use walkdir::WalkDir;

fn main() -> anyhow::Result<()> {
    let args = get_args();

    let is_dir =
        args.input_path.metadata().with_context(|| anyhow!("{:?}", args.input_path))?.is_dir();

    if let Some(output_path) = args.output_path.as_deref() {
        if is_dir {
            match output_path.metadata() {
                Ok(metadata) => {
                    if !metadata.is_dir() {
                        return Err(anyhow!("{output_path:?} is not a directory.",));
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir_all(output_path)
                        .with_context(|| anyhow!("{:?}", output_path))?;
                },
                Err(error) => {
                    return Err(error).with_context(|| anyhow!("{:?}", output_path));
                },
            }
        } else if output_path.is_dir() {
            return Err(anyhow!("{output_path:?} is a directory."));
        }
    }

    let sc: Arc<Mutex<Scanner<io::Stdin, U8>>> = Arc::new(Mutex::new(Scanner::new2(io::stdin())));
    let overwriting: Arc<Mutex<u8>> = Arc::new(Mutex::new(0));

    if is_dir {
        let mut image_paths = Vec::new();

        for dir_entry in WalkDir::new(args.input_path.as_path()).into_iter().filter_map(|e| e.ok())
        {
            if dir_entry.metadata().with_context(|| anyhow!("{dir_entry:?}"))?.is_dir() {
                continue;
            }

            let p = dir_entry.into_path();

            if let Some(extension) = p.extension() {
                if let Some(extension) = extension.to_str() {
                    let mut allow_extensions = vec!["jpg", "jpeg", "png"];

                    if args.allow_gif {
                        allow_extensions.push("gif");
                    }

                    if extension
                        .eq_ignore_ascii_case_with_lowercase_multiple(&allow_extensions)
                        .is_some()
                    {
                        image_paths.push(p);
                    }
                }
            }
        }

        if args.single_thread {
            for image_path in image_paths {
                let output_path = match args.output_path.as_ref() {
                    Some(output_path) => {
                        let p =
                            pathdiff::diff_paths(&image_path, args.input_path.as_path()).unwrap();

                        let output_path = output_path.join(p);

                        Some(output_path)
                    },
                    None => None,
                };

                interlacing(
                    args.allow_gif,
                    args.remain_profile,
                    args.force,
                    &sc,
                    &overwriting,
                    image_path.as_path(),
                    output_path.as_deref(),
                )?;
            }
        } else {
            let cpus = num_cpus::get();

            let pool = ThreadPool::new(cpus * 2);

            for image_path in image_paths {
                let sc = sc.clone();
                let overwriting = overwriting.clone();
                let output_path = match args.output_path.as_ref() {
                    Some(output_path) => {
                        let p =
                            pathdiff::diff_paths(&image_path, args.input_path.as_path()).unwrap();

                        let output_path = output_path.join(p);

                        Some(output_path)
                    },
                    None => None,
                };

                pool.execute(move || {
                    if let Err(error) = interlacing(
                        args.allow_gif,
                        args.remain_profile,
                        args.force,
                        &sc,
                        &overwriting,
                        image_path.as_path(),
                        output_path.as_deref(),
                    ) {
                        eprintln!("{error:?}");
                        io::stderr().flush().unwrap();
                    }
                });
            }

            pool.join();
        }
    } else {
        interlacing(
            args.allow_gif,
            args.remain_profile,
            args.force,
            &sc,
            &overwriting,
            args.input_path,
            args.output_path.as_ref(),
        )?;
    }

    Ok(())
}

fn interlacing<IP: AsRef<Path>, OP: AsRef<Path>>(
    allow_gif: bool,
    remain_profile: bool,
    force: bool,
    sc: &Arc<Mutex<Scanner<io::Stdin, U8>>>,
    overwriting: &Arc<Mutex<u8>>,
    input_path: IP,
    output_path: Option<OP>,
) -> anyhow::Result<()> {
    let input_path = input_path.as_ref();

    let input_image_resource = image_convert::ImageResource::from_path(input_path);

    let input_identify = image_convert::identify_ping(&input_image_resource)
        .with_context(|| anyhow!("{input_path:?}"))?;

    match input_identify.interlace {
        image_convert::InterlaceType::No | image_convert::InterlaceType::Undefined => {
            let allow_interlacing = match input_identify.format.as_str() {
                "JPEG" | "PNG" => true,
                "GIF" => allow_gif,
                _ => false,
            };

            if allow_interlacing {
                let mut output = None;

                let input_identify =
                    image_convert::identify_read(&mut output, &input_image_resource)
                        .with_context(|| anyhow!("{input_path:?}"))?;

                match output {
                    Some(mut magic_wand) => {
                        magic_wand.set_interlace_scheme(image_convert::InterlaceType::Line)?;

                        if !remain_profile {
                            magic_wand.profile_image("*", None)?;
                        }

                        let output_path = match output_path.as_ref().map(|p| p.as_ref()) {
                            Some(output_path) => {
                                if output_path.exists() {
                                    if !force {
                                        let mutex_guard = overwriting.lock().unwrap();

                                        loop {
                                            print!(
                                                "{output_path:?} exists, do you want to overwrite \
                                                 it? [Y/N] ",
                                            );
                                            io::stdout()
                                                .flush()
                                                .with_context(|| anyhow!("stdout"))?;

                                            match sc
                                                .lock()
                                                .unwrap()
                                                .next_line()
                                                .with_context(|| anyhow!("stdout"))?
                                            {
                                                Some(token) => {
                                                    match token.to_ascii_uppercase().as_str() {
                                                        "Y" => {
                                                            break;
                                                        },
                                                        "N" => {
                                                            return Ok(());
                                                        },
                                                        _ => {
                                                            continue;
                                                        },
                                                    }
                                                },
                                                None => {
                                                    return Ok(());
                                                },
                                            }
                                        }

                                        drop(mutex_guard);
                                    }
                                } else {
                                    let dir_path = output_path.parent().unwrap();

                                    fs::create_dir_all(dir_path)
                                        .with_context(|| anyhow!("{dir_path:?}"))?;
                                }

                                output_path
                            },
                            None => input_path,
                        };

                        let temp = magic_wand.write_image_blob(input_identify.format.as_str())?;

                        fs::write(output_path, temp)?;

                        let mutex_guard = overwriting.lock().unwrap();

                        println!("{:?} has been interlaced.", output_path.canonicalize().unwrap());
                        io::stdout().flush()?;

                        drop(mutex_guard);
                    },
                    None => unreachable!(),
                }
            }
        },
        _ => (),
    }

    Ok(())
}
