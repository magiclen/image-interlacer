Image Interlacer
====================

[![CI](https://github.com/magiclen/image-interlacer/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/image-interlacer/actions/workflows/ci.yml)

It helps you interlace an image or multiple images for web-page usage.

## Help

```
EXAMPLES:
image-interlacer /path/to/image                           # Check /path/to/image and make it interlaced
image-interlacer /path/to/folder                          # Check /path/to/folder and make images inside it interlaced
image-interlacer /path/to/image  -o /path/to/image2       # Check /path/to/image and make it interlaced, and save it to /path/to/image2
image-interlacer /path/to/folder -o /path/to/folder2      # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2
image-interlacer /path/to/folder -o /path/to/folder2 -f   # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2 without overwriting checks
image-interlacer /path/to/folder --allow-gif -r           # Check /path/to/folder and make images inside it including GIF images interlaced and also remain their profiles

Usage: image-interlacer [OPTIONS] <INPUT_PATH>

Arguments:
  <INPUT_PATH>  Assign an image or a directory for image interlacing. It should be a path of a file or a directory

Options:
  -o, --output-path <OUTPUT_PATH>  Assign a destination of your generated files. It should be a path of a directory or a file depending on your input path [aliases: output]
  -s, --single-thread              Use only one thread
  -f, --force                      Force to overwrite files
      --allow-gif                  Allow to do GIF interlacing
  -r, --remain-profile             Remain the profiles of all images
  -h, --help                       Print help
  -V, --version                    Print version
```

## License

[MIT](LICENSE)