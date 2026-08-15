mod cli;

use std::{
    fmt, fs, io,
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
use threadpool::ThreadPool;
use walkdir::WalkDir;

const ALLOW_EXTENSIONS: [&str; 3] = ["jpg", "jpeg", "png"];
const ALLOW_EXTENSIONS_WITH_GIF: [&str; 4] = ["jpg", "jpeg", "png", "gif"];

/// The options which decide how an image is interlaced.
#[derive(Debug, Clone, Copy)]
struct Flags {
    allow_gif:      bool,
    remain_profile: bool,
    force:          bool,
}

fn report_error(console_lock: &Mutex<()>, error: &anyhow::Error) {
    let _console_lock = console_lock.lock().unwrap();
    let mut stderr = io::stderr().lock();

    let _ = writeln!(stderr, "{error:?}");
    let _ = stderr.flush();
}

/// Prints a message to stdout. A closed pipe must not take the whole program down, so a failed write is dropped just like the ones to stderr.
fn report_message(console_lock: &Mutex<()>, message: fmt::Arguments<'_>) {
    let _console_lock = console_lock.lock().unwrap();
    let mut stdout = io::stdout().lock();

    let _ = writeln!(stdout, "{message}");
    let _ = stdout.flush();
}

/// Keeps ImageMagick from spreading a single operation over every core, because in this mode the thread pool is what parallelizes the work.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn limit_magick_threads() -> anyhow::Result<()> {
    use image_convert::magick_rust::{MagickWand, ResourceType};

    MagickWand::set_resource_limit(ResourceType::Thread, 1)
        .with_context(|| anyhow!("ImageMagick thread limit"))
}

/// `set_resource_limit` is not available on this platform, so ImageMagick keeps deciding its thread count on its own.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn limit_magick_threads() -> anyhow::Result<()> {
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let CLIArgs {
        mut input_path,
        output_path,
        single_thread,
        force,
        allow_gif,
        remain_profile,
    } = get_args();

    let flags = Flags {
        allow_gif,
        remain_profile,
        force,
    };

    // A symlink named on the command line is an alias for the real image, and renaming over it would replace the link itself instead of updating what it points at.
    if input_path
        .symlink_metadata()
        .with_context(|| anyhow!("{input_path:?}"))?
        .file_type()
        .is_symlink()
    {
        input_path = input_path.canonicalize().with_context(|| anyhow!("{input_path:?}"))?;
    }

    let is_dir = input_path.metadata().with_context(|| anyhow!("{input_path:?}"))?.is_dir();

    if let Some(output_path) = output_path.as_deref() {
        if is_dir {
            match output_path.metadata() {
                Ok(metadata) => {
                    if !metadata.is_dir() {
                        return Err(anyhow!("{output_path:?} is not a directory."));
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir_all(output_path).with_context(|| anyhow!("{output_path:?}"))?;
                },
                Err(error) => {
                    return Err(error).with_context(|| anyhow!("{output_path:?}"));
                },
            }
        } else if output_path.is_dir() {
            return Err(anyhow!("{output_path:?} is a directory."));
        }
    }

    // ImageMagick has to be initialized once before any wand is used. Doing it here keeps it on the main thread and lets the thread limit below apply to a ready environment.
    image_convert::start_call_once();

    let console_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    let error_count = Arc::new(AtomicUsize::new(0));

    if is_dir {
        let mut image_paths = Vec::new();

        for dir_entry in WalkDir::new(input_path.as_path()) {
            let dir_entry = match dir_entry {
                Ok(dir_entry) => dir_entry,
                Err(error) => {
                    // Dropping the entry silently would hide a whole unreadable subtree and still report success.
                    report_error(&console_lock, &anyhow::Error::new(error));
                    error_count.fetch_add(1, Ordering::Relaxed);

                    continue;
                },
            };

            // `file_type` reuses what the directory listing already reported, so it costs no syscall.
            // A symlink either duplicates a file which is walked on its own or points outside the requested tree, and neither is this program's business.
            if !dir_entry.file_type().is_file() {
                continue;
            }

            let p = dir_entry.into_path();

            if let Some(extension) = p.extension().and_then(|extension| extension.to_str()) {
                if is_allowed_extension(extension, allow_gif) {
                    image_paths.push(p);
                }
            }
        }

        if single_thread {
            for image_path in image_paths {
                interlace_entry(
                    flags,
                    &console_lock,
                    &error_count,
                    input_path.as_path(),
                    output_path.as_deref(),
                    image_path.as_path(),
                );
            }
        } else {
            let cpus = thread::available_parallelism().map(|cpus| cpus.get()).unwrap_or(1);

            limit_magick_threads()?;

            // ImageMagick parallelizes each operation internally, so one worker per core keeps the throughput while holding far fewer decoded images at once.
            let pool = ThreadPool::new(cpus);

            // Every job reads the same two roots, so they are shared instead of being copied into each of them.
            let input_root: Arc<Path> = Arc::from(input_path.as_path());
            let output_root: Option<Arc<Path>> = output_path.as_deref().map(Arc::from);

            for image_path in image_paths {
                let console_lock = console_lock.clone();
                let error_count = error_count.clone();
                let input_root = input_root.clone();
                let output_root = output_root.clone();

                pool.execute(move || {
                    interlace_entry(
                        flags,
                        &console_lock,
                        &error_count,
                        &input_root,
                        output_root.as_deref(),
                        image_path.as_path(),
                    );
                });
            }

            pool.join();
        }
    } else {
        interlacing(flags, &console_lock, input_path.as_path(), output_path.as_deref())?;
    }

    let error_count = error_count.load(Ordering::Relaxed);

    if error_count > 0 {
        // Workers only report to stderr, so the exit code has to be set here to match the single-thread mode.
        return Err(anyhow!("{error_count} path(s) failed."));
    }

    Ok(())
}

/// Interlaces one image of a directory tree. A failure is reported and counted instead of being returned, because bailing out would leave the remaining images unhandled and the thread pool holding jobs which nothing waits for.
fn interlace_entry(
    flags: Flags,
    console_lock: &Mutex<()>,
    error_count: &AtomicUsize,
    input_root: &Path,
    output_root: Option<&Path>,
    image_path: &Path,
) {
    let result = map_output_path(input_root, output_root, image_path).and_then(|output_path| {
        interlacing(flags, console_lock, image_path, output_path.as_deref())
    });

    if let Err(error) = result {
        report_error(console_lock, &error);
        error_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Checks whether a file extension belongs to an image format this program can interlace.
fn is_allowed_extension(extension: &str, allow_gif: bool) -> bool {
    let allow_extensions: &[&str] =
        if allow_gif { &ALLOW_EXTENSIONS_WITH_GIF } else { &ALLOW_EXTENSIONS };

    allow_extensions.iter().any(|allow_extension| extension.eq_ignore_ascii_case(allow_extension))
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

/// Reads an answer to the overwrite prompt. `None` means the answer was not understood and the prompt has to be repeated.
fn parse_overwrite_answer(answer: &str) -> Option<bool> {
    match answer.trim().to_ascii_uppercase().as_str() {
        "Y" => Some(true),
        "N" => Some(false),
        _ => None,
    }
}

/// Asks whether an existing file may be replaced. The console lock is held for the whole prompt so that other threads cannot interleave their output.
fn confirm_overwrite(console_lock: &Mutex<()>, output_path: &Path) -> anyhow::Result<bool> {
    let _console_lock = console_lock.lock().unwrap();
    let mut stdout = io::stdout().lock();
    let stdin = io::stdin();
    let mut answer = String::new();

    loop {
        let _ = write!(stdout, "{output_path:?} exists, do you want to overwrite it? [Y/N] ");
        let _ = stdout.flush();

        answer.clear();

        // Nothing left to read, such as a closed stdin, leaves the file alone.
        if stdin.read_line(&mut answer).with_context(|| anyhow!("stdin"))? == 0 {
            return Ok(false);
        }

        if let Some(overwrite) = parse_overwrite_answer(answer.as_str()) {
            return Ok(overwrite);
        }
    }
}

/// Writes data through a sibling temporary file and renames it into place, so an interrupted write cannot destroy the original image.
fn write_atomically(output_path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let mut temp_path = output_path.as_os_str().to_os_string();

    temp_path.push(format!(".{}.tmp", process::id()));

    let temp_path = PathBuf::from(temp_path);

    let permissions = match fs::symlink_metadata(output_path) {
        // The rename below would replace the link itself rather than write through it. Callers are expected to have settled this already, so this is the last-line guard.
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(anyhow!("{output_path:?} is a symbolic link."));
        },
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| anyhow!("{output_path:?}")),
    };

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

fn interlacing(
    flags: Flags,
    console_lock: &Mutex<()>,
    input_path: &Path,
    output_path: Option<&Path>,
) -> anyhow::Result<()> {
    // Handing ImageMagick a path means running it through `to_string_lossy` first, which turns a name that is not UTF-8 into one that does not exist. Reading the bytes here keeps the file this program was pointed at.
    let input_data = fs::read(input_path).with_context(|| anyhow!("{input_path:?}"))?;

    let input_image_resource = image_convert::ImageResource::Data(input_data);

    let input_identify = image_convert::identify_ping(&input_image_resource)
        .with_context(|| anyhow!("{input_path:?}"))?;

    if !is_allowed_format(input_identify.format.as_str(), flags.allow_gif) {
        report_message(
            console_lock,
            format_args!("{input_path:?} is not an interlaceable format."),
        );

        return Ok(());
    }

    if !matches!(
        input_identify.interlace,
        image_convert::InterlaceType::No | image_convert::InterlaceType::Undefined
    ) {
        // Saying nothing would leave an output directory quietly missing this image.
        report_message(console_lock, format_args!("{input_path:?} is already interlaced."));

        return Ok(());
    }

    // The destination is settled before decoding, so that declining an overwrite does not waste a full decode.
    let output_path = match output_path {
        Some(output_path) => {
            match fs::symlink_metadata(output_path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(anyhow!("{output_path:?} is a symbolic link."));
                },
                Ok(_) => {
                    if !flags.force && !confirm_overwrite(console_lock, output_path)? {
                        return Ok(());
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let Some(dir_path) =
                        output_path.parent().filter(|dir_path| !dir_path.as_os_str().is_empty())
                    {
                        fs::create_dir_all(dir_path).with_context(|| anyhow!("{dir_path:?}"))?;
                    }
                },
                Err(error) => return Err(error).with_context(|| anyhow!("{output_path:?}")),
            }

            output_path
        },
        None => input_path,
    };

    let mut output = None;

    let input_identify = image_convert::identify_read(&mut output, &input_image_resource)
        .with_context(|| anyhow!("{input_path:?}"))?;

    // The wand holds an image of its own now, so keeping the file bytes would only add to what the encoding below needs.
    drop(input_image_resource);

    let mut magic_wand = output.expect("identify_read fills the output in whenever it succeeds");

    magic_wand
        .set_interlace_scheme(image_convert::InterlaceType::Line)
        .with_context(|| anyhow!("{input_path:?}"))?;

    if !flags.remain_profile {
        // `profile_image` only touches the frame the iterator points at, so every frame of an animation has to be visited.
        magic_wand.reset_iterator();

        while magic_wand.next_image() {
            magic_wand.profile_image("*", None).with_context(|| anyhow!("{input_path:?}"))?;
        }
    }

    // `write_image_blob` keeps only the frame the iterator points at, which would turn an animation into a still image.
    let temp = magic_wand
        .write_images_blob(input_identify.format.as_str())
        .with_context(|| anyhow!("{input_path:?}"))?;

    // Unlike `write_image_blob`, `write_images_blob` does not check the pointer it gets back, so a failed encode arrives as an empty vec rather than an error.
    if temp.is_empty() {
        return Err(anyhow!("{input_path:?} could not be encoded."));
    }

    write_atomically(output_path, &temp)?;

    match output_path.canonicalize() {
        // The file is already written at this point, so a failure here must not be fatal.
        Ok(canonicalized_path) => report_message(
            console_lock,
            format_args!("{canonicalized_path:?} has been interlaced."),
        ),
        Err(_) => {
            report_message(console_lock, format_args!("{output_path:?} has been interlaced."))
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    // A 4x4 GIF with two frames, used to check that an animation survives interlacing.
    const ANIMATED_GIF: [u8; 95] = [
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x04, 0x00, 0x04, 0x00, 0xF0, 0x00, 0x00, 0xFF, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x21, 0xFF, 0x0B, 0x4E, 0x45, 0x54, 0x53, 0x43, 0x41, 0x50, 0x45,
        0x32, 0x2E, 0x30, 0x03, 0x01, 0x00, 0x00, 0x00, 0x21, 0xF9, 0x04, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x04, 0x00, 0x00, 0x02, 0x04, 0x84, 0x8F,
        0x09, 0x05, 0x00, 0x21, 0xF9, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00,
        0x00, 0x04, 0x00, 0x04, 0x00, 0x80, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x02, 0x04, 0x84,
        0x8F, 0x09, 0x05, 0x00, 0x3B,
    ];

    /// A directory under the system temporary directory which removes itself when it goes out of scope.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);

            let path = env::temp_dir().join(format!(
                "image-interlacer-{}-{}",
                process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));

            fs::create_dir_all(path.as_path()).unwrap();

            TempDir(path)
        }

        fn join(&self, file_name: &str) -> PathBuf {
            self.0.join(file_name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0.as_path());
        }
    }

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

    #[test]
    fn overwrite_answers() {
        assert_eq!(Some(true), parse_overwrite_answer("y"));
        assert_eq!(Some(true), parse_overwrite_answer("Y"));
        assert_eq!(Some(true), parse_overwrite_answer("y "));

        assert_eq!(Some(false), parse_overwrite_answer("n"));
        assert_eq!(Some(false), parse_overwrite_answer("N"));
        assert_eq!(Some(false), parse_overwrite_answer(" N"));

        assert_eq!(None, parse_overwrite_answer(""));
        assert_eq!(None, parse_overwrite_answer("maybe"));
    }

    #[test]
    fn write_atomically_creates_a_new_file() {
        let temp_dir = TempDir::new();
        let output_path = temp_dir.join("image.png");

        write_atomically(output_path.as_path(), b"interlaced").unwrap();

        assert_eq!(b"interlaced".to_vec(), fs::read(output_path.as_path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn write_atomically_keeps_the_mode_of_the_replaced_file() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new();
        let output_path = temp_dir.join("image.png");

        fs::write(output_path.as_path(), b"original").unwrap();
        fs::set_permissions(output_path.as_path(), fs::Permissions::from_mode(0o600)).unwrap();

        write_atomically(output_path.as_path(), b"interlaced").unwrap();

        assert_eq!(b"interlaced".to_vec(), fs::read(output_path.as_path()).unwrap());
        assert_eq!(
            0o600,
            fs::metadata(output_path.as_path()).unwrap().permissions().mode() & 0o777
        );
    }

    #[test]
    fn interlacing_keeps_every_frame_of_an_animation() {
        let temp_dir = TempDir::new();
        let input_path = temp_dir.join("animated.gif");

        fs::write(input_path.as_path(), ANIMATED_GIF).unwrap();

        let console_lock = Mutex::new(());

        let flags = Flags {
            allow_gif: true, remain_profile: false, force: true
        };

        interlacing(flags, &console_lock, input_path.as_path(), None).unwrap();

        let mut output = None;

        let identify = image_convert::identify_read(
            &mut output,
            &image_convert::ImageResource::Data(fs::read(input_path.as_path()).unwrap()),
        )
        .unwrap();

        assert_eq!(2, output.unwrap().get_number_images());
        assert_eq!(image_convert::InterlaceType::GIF, identify.interlace);
    }
}
