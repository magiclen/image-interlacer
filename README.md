Image Interlacer
====================

[![Build Status](https://travis-ci.org/magiclen/image-interlacer.svg?branch=master)](https://travis-ci.org/magiclen/image-interlacer)

It helps you interlace an image or multiple images for web-page usage.

## Help

```
EXAMPLES:
  image-interlacer /path/to/image                           # Check /path/to/image and make it interlaced
  image-interlacer /path/to/folder                          # Check /path/to/folder and make images inside it interlaced
  image-interlacer /path/to/image  -o /path/to/image2       # Check /path/to/image and make it interlaced, and save it
to /path/to/image2
  image-interlacer /path/to/folder -o /path/to/folder2      # Check /path/to/folder and make images inside it
interlaced, and save them to /path/to/folder2
  image-interlacer /path/to/folder -o /path/to/folder2 -f   # Check /path/to/folder and make images inside it
interlaced, and save them to /path/to/folder2 without overwriting checks
  image-interlacer /path/to/folder --allow-gif -r           # Check /path/to/folder and make images inside it including
GIF images interlaced and also remain their profiles

USAGE:
    image-interlacer [FLAGS] [OPTIONS] <INPUT_PATH>

FLAGS:
        --allow-gif         Allows to do GIF interlacing
    -f, --force             Forces to overwrite files
    -r, --remain-profile    Remains the profiles of all images
    -s, --single-thread     Uses only one thread
    -h, --help              Prints help information
    -V, --version           Prints version information

OPTIONS:
    -o, --output <OUTPUT_PATH>    Assigns a destination of your generated files. It should be a path of a directory or a
                                  file depending on your input path

ARGS:
    <INPUT_PATH>    Assigns an image or a directory for image interlacing. It should be a path of a file or a
                    directory
```

## License

[MIT](LICENSE)