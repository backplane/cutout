# cutout

A command-line tool for extracting rectangular regions from images.

## Overview

`cutout` allows you to define one or more rectangular regions and extract them from images. It supports multiple capture specifications per image and can work with different coordinate systems (top-left or bottom-left origin).

## Usage

```sh
cutout [OPTIONS] --capture <SPEC> <INPUTS>...
```

### Arguments

- `<INPUTS>...` - One or more input image files to process

### Options

- `-c, --capture <SPEC>` - Capture specification (can be repeated for multiple regions)

  - Format: `<name>:<x>x<y>:<width>x<height>`
  - Example: `left:200x300:1200x1850`

- `--origin <ORIGIN>` - Coordinate system origin (default: `tl`)
  - `tl`, `top-left`, or `top_left` - Standard image coordinates (0,0 at top-left)
  - `bl`, `bottom-left`, or `bottom_left` - Y coordinates measured from bottom

- `-v, --verbose` - Enable verbose output with timing information

- `--dry-run` - Validate capture specifications without processing images

### Output

For each input image and capture specification, the tool creates an output file named:

```
<basename>_<capture_name>.<extension>
```

For example, processing `photo.jpg` with capture name `left` produces `photo_left.jpg`.

## Examples

### Extract a single region from an image

```sh
cutout --capture "center:100x100:200x200" image.jpg
```

This extracts a 200×200 pixel region starting at coordinates (100, 100) and saves it as `image_center.jpg`.

### Extract multiple regions from the same image

```sh
cutout \
  --capture "left:0x0:500x1000" \
  --capture "right:500x0:500x1000" \
  image.jpg
```

This splits the image into left and right halves, creating `image_left.jpg` and `image_right.jpg`.

### Process multiple images with the same capture specifications

```sh
cutout --capture "header:0x0:1920x200" *.png
```

This extracts the top 200 pixels from all PNG files in the current directory.

### Use bottom-left coordinate system

```sh
cutout --origin bl --capture "bottom:0x0:800x100" chart.png
```

This extracts a 800×100 pixel region from the bottom of the image (useful for charts and plots where measurements are naturally from the bottom).

### Complex example with multiple captures

```sh
cutout \
  --capture "header:0x0:1920x100" \
  --capture "footer:0x980:1920x100" \
  --capture "content:200x100:1520x880" \
  page.jpg
```

This extracts header, footer, and main content regions from a page layout.

### Validate capture specifications before processing

```sh
cutout --dry-run --capture "left:0x0:500x1000" image.jpg
```

This validates that the capture specification is valid for the given image without actually processing it.

### Enable verbose output with timing

```sh
cutout --verbose --capture "region:100x100:200x200" *.jpg
```

This processes images with verbose output showing decode and crop/save timing for each file.

## Coordinate Systems

### Top-Left Origin (default)

In the standard coordinate system, (0, 0) is at the top-left corner of the image:

- X increases going right
- Y increases going down

### Bottom-Left Origin

When using `--origin bl`, (0, 0) is at the bottom-left corner:

- X increases going right
- Y increases going up

This is useful when working with charts, graphs, or other images where measurements are naturally from the bottom.

## Error Handling

The tool validates all coordinates before processing and will report clear errors if:

- Capture specifications are malformed
- Coordinates are outside image bounds
- Width or height is zero
- Images cannot be opened or saved

## Supported Image Formats

Supports all formats provided by the `image` crate, including:

- JPEG
- PNG
- GIF
- BMP
- TIFF
- WebP
- And more

Output format is inferred from the input file extension.
