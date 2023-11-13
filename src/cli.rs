use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser};
use concat_with::concat_line;
use terminal_size::terminal_size;

const APP_NAME: &str = "Image Interlacer";
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

const AFTER_HELP: &str = "Enjoy it! https://magiclen.org";

const APP_ABOUT: &str = concat!(
    "It helps you interlace an image or multiple images for web-page usage.\n\nEXAMPLES:\n",
    concat_line!(prefix "image-interlacer ",
        "/path/to/image                           # Check /path/to/image and make it interlaced",
        "/path/to/folder                          # Check /path/to/folder and make images inside it interlaced",
        "/path/to/image  -o /path/to/image2       # Check /path/to/image and make it interlaced, and save it to /path/to/image2",
        "/path/to/folder -o /path/to/folder2      # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2",
        "/path/to/folder -o /path/to/folder2 -f   # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2 without overwriting checks",
        "/path/to/folder --allow-gif -r           # Check /path/to/folder and make images inside it including GIF images interlaced and also remain their profiles",
    )
);

#[derive(Debug, Parser)]
#[command(name = APP_NAME)]
#[command(term_width = terminal_size().map(|(width, _)| width.0 as usize).unwrap_or(0))]
#[command(version = CARGO_PKG_VERSION)]
#[command(author = CARGO_PKG_AUTHORS)]
#[command(after_help = AFTER_HELP)]
pub struct CLIArgs {
    #[arg(value_hint = clap::ValueHint::AnyPath)]
    #[arg(help = "Assign an image or a directory for image interlacing. It should be a path of \
                  a file or a directory")]
    pub input_path:     PathBuf,
    #[arg(short, long, visible_alias = "output")]
    #[arg(value_hint = clap::ValueHint::AnyPath)]
    #[arg(help = "Assign a destination of your generated files. It should be a path of a \
                  directory or a file depending on your input path")]
    pub output_path:    Option<PathBuf>,
    #[arg(short, long)]
    #[arg(help = "Use only one thread")]
    pub single_thread:  bool,
    #[arg(short, long)]
    #[arg(help = "Force to overwrite files")]
    pub force:          bool,
    #[arg(long)]
    #[arg(help = "Allow to do GIF interlacing")]
    pub allow_gif:      bool,
    #[arg(short, long)]
    #[arg(help = "Remain the profiles of all images")]
    pub remain_profile: bool,
}

pub fn get_args() -> CLIArgs {
    let args = CLIArgs::command();

    let about = format!("{APP_NAME} {CARGO_PKG_VERSION}\n{CARGO_PKG_AUTHORS}\n{APP_ABOUT}");

    let args = args.about(about);

    let matches = args.get_matches();

    match CLIArgs::from_arg_matches(&matches) {
        Ok(args) => args,
        Err(err) => {
            err.exit();
        },
    }
}
