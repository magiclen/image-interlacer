mod cli;

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use anyhow::{Context, anyhow};
use cli::*;
use scanner_rust::Scanner;
use str_utils::EqIgnoreAsciiCaseMultiple;
use threadpool::ThreadPool;
use walkdir::WalkDir;

const ALLOW_EXTENSIONS: [&str; 3] = ["jpg", "jpeg", "png"];
const ALLOW_EXTENSIONS_WITH_GIF: [&str; 4] = ["jpg", "jpeg", "png", "gif"];

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

    let sc: Arc<Mutex<Scanner<io::Stdin, 8>>> = Arc::new(Mutex::new(Scanner::new2(io::stdin())));
    let console_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    let error_count = Arc::new(AtomicUsize::new(0));

    if is_dir {
        let mut image_paths = Vec::new();

        for dir_entry in WalkDir::new(args.input_path.as_path()) {
            let dir_entry = match dir_entry {
                Ok(dir_entry) => dir_entry,
                Err(error) => {
                    // Dropping the entry silently would hide a whole unreadable subtree and still report success.
                    eprintln!("{error}");
                    error_count.fetch_add(1, Ordering::Relaxed);

                    continue;
                },
            };

            // `file_type` reuses what the directory listing already reported, so it costs no syscall.
            if dir_entry.file_type().is_dir() {
                continue;
            }

            let p = dir_entry.into_path();

            if let Some(extension) = p.extension().and_then(|extension| extension.to_str()) {
                if is_allowed_extension(extension, args.allow_gif) {
                    image_paths.push(p);
                }
            }
        }

        if args.single_thread {
            for image_path in image_paths {
                let output_path = map_output_path(
                    args.input_path.as_path(),
                    args.output_path.as_deref(),
                    image_path.as_path(),
                )?;

                interlacing(
                    args.allow_gif,
                    args.remain_profile,
                    args.force,
                    &sc,
                    &console_lock,
                    image_path.as_path(),
                    output_path.as_deref(),
                )?;
            }
        } else {
            let cpus = thread::available_parallelism().map(|cpus| cpus.get()).unwrap_or(1);

            // ImageMagick parallelizes each operation internally, so one worker per core keeps the throughput while holding far fewer decoded images at once.
            let pool = ThreadPool::new(cpus);

            for image_path in image_paths {
                let sc = sc.clone();
                let console_lock = console_lock.clone();
                let error_count = error_count.clone();
                let output_path = map_output_path(
                    args.input_path.as_path(),
                    args.output_path.as_deref(),
                    image_path.as_path(),
                )?;

                pool.execute(move || {
                    if let Err(error) = interlacing(
                        args.allow_gif,
                        args.remain_profile,
                        args.force,
                        &sc,
                        &console_lock,
                        image_path.as_path(),
                        output_path.as_deref(),
                    ) {
                        eprintln!("{error:?}");
                        io::stderr().flush().unwrap();

                        error_count.fetch_add(1, Ordering::Relaxed);
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
            &console_lock,
            args.input_path,
            args.output_path.as_ref(),
        )?;
    }

    let error_count = error_count.load(Ordering::Relaxed);

    if error_count > 0 {
        // Workers only report to stderr, so the exit code has to be set here to match the single-thread mode.
        return Err(anyhow!("{error_count} path(s) failed."));
    }

    Ok(())
}

/// Checks whether a file extension belongs to an image format this program can interlace.
fn is_allowed_extension(extension: &str, allow_gif: bool) -> bool {
    let allow_extensions: &[&str] =
        if allow_gif { &ALLOW_EXTENSIONS_WITH_GIF } else { &ALLOW_EXTENSIONS };

    extension.eq_ignore_ascii_case_with_lowercase_multiple(allow_extensions).is_some()
}

/// Checks whether an ImageMagick format name belongs to an image format this program can interlace.
fn is_allowed_format(format: &str, allow_gif: bool) -> bool {
    match format {
        "JPEG" | "PNG" => true,
        "GIF" => allow_gif,
        _ => false,
    }
}

/// Maps an image path to its destination under the output root, keeping the directory structure below the input root.
fn map_output_path(
    input_root: &Path,
    output_root: Option<&Path>,
    image_path: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    match output_root {
        Some(output_root) => {
            let relative_path =
                image_path.strip_prefix(input_root).with_context(|| anyhow!("{image_path:?}"))?;

            Ok(Some(output_root.join(relative_path)))
        },
        None => Ok(None),
    }
}

/// Asks whether an existing file may be replaced. The console lock is held for the whole prompt so that other threads cannot interleave their output.
fn confirm_overwrite(
    sc: &Arc<Mutex<Scanner<io::Stdin, 8>>>,
    console_lock: &Arc<Mutex<()>>,
    output_path: &Path,
) -> anyhow::Result<bool> {
    let _console_lock = console_lock.lock().unwrap();

    loop {
        print!("{output_path:?} exists, do you want to overwrite it? [Y/N] ");
        io::stdout().flush().with_context(|| anyhow!("stdout"))?;

        match sc.lock().unwrap().next_line().with_context(|| anyhow!("stdin"))? {
            Some(token) => match token.to_ascii_uppercase().as_str() {
                "Y" => return Ok(true),
                "N" => return Ok(false),
                _ => continue,
            },
            None => return Ok(false),
        }
    }
}

/// Writes data through a sibling temporary file and renames it into place, so an interrupted write cannot destroy the original image.
fn write_atomically(output_path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let mut temp_path = output_path.as_os_str().to_os_string();

    temp_path.push(format!(".{}.tmp", process::id()));

    let temp_path = PathBuf::from(temp_path);

    // The mode of the file being replaced has to be carried over, otherwise overwriting resets it to the default one.
    let permissions = fs::metadata(output_path).ok().map(|metadata| metadata.permissions());

    let result = fs::write(temp_path.as_path(), data)
        .and_then(|_| match permissions {
            Some(permissions) => fs::set_permissions(temp_path.as_path(), permissions),
            None => Ok(()),
        })
        .and_then(|_| fs::rename(temp_path.as_path(), output_path));

    if result.is_err() {
        // A half-written temporary file is useless and would only litter the output directory.
        let _ = fs::remove_file(temp_path.as_path());
    }

    result.with_context(|| anyhow!("{temp_path:?}"))
}

fn interlacing<IP: AsRef<Path>, OP: AsRef<Path>>(
    allow_gif: bool,
    remain_profile: bool,
    force: bool,
    sc: &Arc<Mutex<Scanner<io::Stdin, 8>>>,
    console_lock: &Arc<Mutex<()>>,
    input_path: IP,
    output_path: Option<OP>,
) -> anyhow::Result<()> {
    let input_path = input_path.as_ref();

    let input_image_resource = image_convert::ImageResource::from_path(input_path);

    let input_identify = image_convert::identify_ping(&input_image_resource)
        .with_context(|| anyhow!("{input_path:?}"))?;

    if !matches!(
        input_identify.interlace,
        image_convert::InterlaceType::No | image_convert::InterlaceType::Undefined
    ) {
        return Ok(());
    }

    if !is_allowed_format(input_identify.format.as_str(), allow_gif) {
        return Ok(());
    }

    // The destination is settled before decoding, so that declining an overwrite does not waste a full decode.
    let output_path = match output_path.as_ref().map(|p| p.as_ref()) {
        Some(output_path) => {
            if output_path.exists() {
                if !force && !confirm_overwrite(sc, console_lock, output_path)? {
                    return Ok(());
                }
            } else {
                let dir_path = output_path.parent().unwrap();

                fs::create_dir_all(dir_path).with_context(|| anyhow!("{dir_path:?}"))?;
            }

            output_path
        },
        None => input_path,
    };

    let mut output = None;

    let input_identify = image_convert::identify_read(&mut output, &input_image_resource)
        .with_context(|| anyhow!("{input_path:?}"))?;

    // `identify_read` always fills `output` in before returning successfully.
    let mut magic_wand = output.unwrap();

    magic_wand
        .set_interlace_scheme(image_convert::InterlaceType::Line)
        .with_context(|| anyhow!("{input_path:?}"))?;

    if !remain_profile {
        magic_wand.profile_image("*", None).with_context(|| anyhow!("{input_path:?}"))?;
    }

    let temp = magic_wand
        .write_image_blob(input_identify.format.as_str())
        .with_context(|| anyhow!("{input_path:?}"))?;

    write_atomically(output_path, &temp).with_context(|| anyhow!("{output_path:?}"))?;

    let _console_lock = console_lock.lock().unwrap();

    match output_path.canonicalize() {
        // The file is already written at this point, so a failure here must not be fatal.
        Ok(canonicalized_path) => println!("{canonicalized_path:?} has been interlaced."),
        Err(_) => println!("{output_path:?} has been interlaced."),
    }

    io::stdout().flush().with_context(|| anyhow!("stdout"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_extensions() {
        assert!(is_allowed_extension("jpg", false));
        assert!(is_allowed_extension("JPG", false));
        assert!(is_allowed_extension("jpeg", false));
        assert!(is_allowed_extension("Jpeg", false));
        assert!(is_allowed_extension("png", false));
        assert!(is_allowed_extension("PNG", false));

        assert!(!is_allowed_extension("bmp", false));
        assert!(!is_allowed_extension("webp", false));
    }

    #[test]
    fn allowed_gif_extension() {
        assert!(!is_allowed_extension("gif", false));

        assert!(is_allowed_extension("gif", true));
        assert!(is_allowed_extension("GIF", true));
    }

    #[test]
    fn allowed_formats() {
        assert!(is_allowed_format("JPEG", false));
        assert!(is_allowed_format("PNG", false));

        assert!(!is_allowed_format("BMP", false));
        assert!(!is_allowed_format("WEBP", false));
    }

    #[test]
    fn allowed_gif_format() {
        assert!(!is_allowed_format("GIF", false));

        assert!(is_allowed_format("GIF", true));
    }

    #[test]
    fn output_path_keeps_directory_structure() {
        let output_path = map_output_path(
            Path::new("/input"),
            Some(Path::new("/output")),
            Path::new("/input/a/b/image.png"),
        )
        .unwrap();

        assert_eq!(Some(PathBuf::from("/output/a/b/image.png")), output_path);
    }

    #[test]
    fn output_path_is_absent_without_an_output_root() {
        let output_path =
            map_output_path(Path::new("/input"), None, Path::new("/input/image.png")).unwrap();

        assert_eq!(None, output_path);
    }
}
