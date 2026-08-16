Image Interlacer
====================

[![CI](https://github.com/magiclen/image-interlacer/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/image-interlacer/actions/workflows/ci.yml)

It helps you interlace an image or multiple images for web-page usage.

## Installation

This program links against **ImageMagick 7** built with **HDRI** enabled, so the library and its development headers have to be installed before building. Distribution packages are often ImageMagick 6, or are built without HDRI, in which case building from source is the reliable route.

#### Debian / Ubuntu

```bash
sudo apt install libwebp-dev
wget https://download.imagemagick.org/archive/ImageMagick.tar.gz
tar xf ImageMagick.tar.gz
cd ImageMagick-*
./configure --enable-hdri
make -j$(nproc)
sudo make install
sudo ldconfig
```

#### macOS

```bash
brew install imagemagick
```

Once ImageMagick is in place,

```bash
cargo install image-interlacer
```

The [CI workflow](.github/workflows/ci.yml) is a working reference for both platforms, and the [Makefile](Makefile) builds a statically linked musl binary.

## Note

An image is decoded, switched to an interlaced scheme and encoded again. For PNG and GIF that round trip is lossless, but for JPEG it is not: the result is re-compressed rather than rearranged in place, so running this program on the same JPEG over and over degrades it. The JPEG is re-compressed with the quality and the subsampling ImageMagick reads from the original, so nothing beyond that round trip is thrown away.

An animated PNG (APNG) is skipped and left alone, because ImageMagick reads its first frame only and interlacing it would throw the animation away.

Unless `--remain-metadata` is given, the metadata is removed. The orientation an image asks for in its metadata is applied to the image itself before that happens, so a photo which was taken sideways does not end up lying on its side.

## Help

```
EXAMPLES:
image-interlacer /path/to/image                           # Check /path/to/image and make it interlaced
image-interlacer /path/to/folder                          # Check /path/to/folder and make images inside it interlaced
image-interlacer /path/to/image  -o /path/to/image2       # Check /path/to/image and make it interlaced, and save it to /path/to/image2
image-interlacer /path/to/folder -o /path/to/folder2      # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2
image-interlacer /path/to/folder -o /path/to/folder2 -f   # Check /path/to/folder and make images inside it interlaced, and save them to /path/to/folder2 without overwriting checks
image-interlacer /path/to/folder --allow-gif -r           # Check /path/to/folder and make images inside it including GIF images interlaced and also remain their metadata

Usage: image-interlacer [OPTIONS] <INPUT_PATH>

Arguments:
  <INPUT_PATH>  Assign an image or a directory for image interlacing. It should be a path of a file or a directory

Options:
  -o, --output-path <OUTPUT_PATH>  Assign a destination of your generated files. It should be a path of a directory or a file depending on your input path [alias: --output]
  -s, --single-thread              Use only one thread
  -f, --force                      Force to overwrite files
      --allow-gif                  Allow to do GIF interlacing
  -r, --remain-metadata            Remain the metadata of all images [alias: --remain-profile]
  -h, --help                       Print help
  -V, --version                    Print version
```

## License

[MIT](LICENSE)