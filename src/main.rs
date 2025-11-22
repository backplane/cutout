use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use clap::{ArgAction, Parser};
use image::{DynamicImage, GenericImageView};
use rayon::prelude::*;

/// A rectangular capture region specification.
///
/// Defines a named rectangular area within an image to be extracted.
/// Coordinates are specified relative to the chosen origin (top-left or bottom-left).
#[derive(Debug, Clone)]
struct CaptureSpec {
    /// Name of the capture region, used in output filename
    name: String,
    /// X coordinate (left edge) in pixels
    x: u32,
    /// Y coordinate in pixels (interpretation depends on origin)
    y: u32,
    /// Width of the region in pixels
    width: u32,
    /// Height of the region in pixels
    height: u32,
}

/// Coordinate system origin for image coordinates.
///
/// Determines how Y coordinates are interpreted:
/// - `TopLeft`: Standard image coordinates where Y increases downward
/// - `BottomLeft`: Mathematical coordinates where Y increases upward
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Y=0 is at the top of the image, Y increases downward
    TopLeft,
    /// Y=0 is at the bottom of the image, Y increases upward
    BottomLeft,
}

impl std::str::FromStr for Origin {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tl" | "top-left" | "top_left" => Ok(Origin::TopLeft),
            "bl" | "bottom-left" | "bottom_left" => Ok(Origin::BottomLeft),
            other => Err(format!(
                "Invalid origin '{other}'. Supported values: tl, bl"
            )),
        }
    }
}

/// Command-line arguments for the cutout tool.
///
/// Extracts rectangular regions from images according to capture specifications.
/// Supports parallel processing of multiple images with multiple capture regions per image.
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "cutout: extract rectangular regions from images",
    long_about = None
)]
struct Cli {
    /// Coordinate origin: 'tl' (top-left) or 'bl' (bottom-left)
    #[arg(
        long,
        value_parser,
        default_value = "tl",
        help = "Coordinate origin: tl (top-left) or bl (bottom-left)"
    )]
    origin: Origin,

    /// A rectangular area to capture. Can be repeated.
    ///
    /// Format: <name>:<x>x<y>:<width>x<height>
    ///
    /// Example: left:200x300:1200x1850
    #[arg(
        long,
        short = 'c',
        value_name = "SPEC",
        action = ArgAction::Append,
        required = true,
        help = "Capture spec: <name>:<x>x<y>:<width>x<height>. Can be repeated."
    )]
    capture: Vec<String>,

    /// Input image files (e.g. *.jpg, *.png, *.tif, *.webp, *.gif, *.bmp)
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    /// Enable verbose output with timing information
    #[arg(
        long,
        short = 'v',
        help = "Enable verbose output with timing information"
    )]
    verbose: bool,

    /// Validate capture specifications without processing images
    #[arg(
        long,
        help = "Validate capture specifications without processing images"
    )]
    dry_run: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Parse capture specs
    let specs: Vec<CaptureSpec> = cli
        .capture
        .iter()
        .map(|s| parse_capture_spec(s))
        .collect::<Result<_>>()?;

    if cli.dry_run {
        // Validate mode: check specs against image dimensions without processing
        eprintln!(
            "Dry run mode: validating {} capture specs against {} images",
            specs.len(),
            cli.inputs.len()
        );
        for spec in &specs {
            eprintln!(
                "  Capture '{}': {}x{} at ({}, {})",
                spec.name, spec.width, spec.height, spec.x, spec.y
            );
        }
        eprintln!();

        for input in &cli.inputs {
            validate_image(input, cli.origin, &specs)?;
        }

        eprintln!("Validation successful. All capture specifications are valid.");
        return Ok(());
    }

    // Process files in parallel
    cli.inputs
        .par_iter()
        .map(|input| {
            process_image(input, cli.origin, &specs, cli.verbose)
                .with_context(|| format!("Failed to process input image: {}", input.display()))
        })
        .collect::<Result<()>>()?;

    Ok(())
}

/// Parse a single capture specification string.
///
/// Format: <name>:<x>x<y>:<width>x<height>
/// Example: left:200x300:1200x1850
fn parse_capture_spec(s: &str) -> Result<CaptureSpec> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!(
            "Invalid capture spec '{s}'. Expected format: <name>:<x>x<y>:<width>x<height>"
        ));
    }

    let name = parts[0].to_string();
    let (x, y) = parse_pair(parts[1], 'x', "x", s)?;
    let (w, h) = parse_pair(parts[2], 'x', "width x height", s)?;

    if w == 0 || h == 0 {
        return Err(anyhow!(
            "Width and height must be positive in capture spec '{s}'"
        ));
    }

    Ok(CaptureSpec {
        name,
        x,
        y,
        width: w,
        height: h,
    })
}

/// Parse a pair of u32 values separated by a given separator character.
fn parse_pair(raw: &str, sep: char, label: &str, original_spec: &str) -> Result<(u32, u32)> {
    let mut parts = raw.split(sep);
    let first = parts
        .next()
        .ok_or_else(|| anyhow!("Missing first {label}"))?;
    let second = parts
        .next()
        .ok_or_else(|| anyhow!("Missing second {label}"))?;

    if parts.next().is_some() {
        return Err(anyhow!(
            "Too many components for {label} in capture spec '{original_spec}'"
        ));
    }

    let a: u32 = first.parse().with_context(|| {
        format!("Failed to parse first {label} value '{first}' in capture spec '{original_spec}'")
    })?;
    let b: u32 = second.parse().with_context(|| {
        format!("Failed to parse second {label} value '{second}' in capture spec '{original_spec}'")
    })?;

    Ok((a, b))
}

/// Convert capture spec coordinates to absolute image coordinates based on origin.
/// Returns (`abs_x`, `abs_y`) in top-left coordinate system.
fn convert_coordinates(
    spec: &CaptureSpec,
    origin: Origin,
    img_width: u32,
    img_height: u32,
) -> Result<(u32, u32)> {
    let abs_x = spec.x;
    let abs_y = match origin {
        Origin::TopLeft => spec.y,
        Origin::BottomLeft => {
            if spec.y > img_height {
                return Err(anyhow!(
                    "Capture '{}' y={} is outside image height={}",
                    spec.name,
                    spec.y,
                    img_height,
                ));
            }
            img_height
                .checked_sub(spec.y)
                .and_then(|v| v.checked_sub(spec.height))
                .ok_or_else(|| {
                    anyhow!(
                        "Capture '{}' (y={}, height={}) is outside image height={}",
                        spec.name,
                        spec.y,
                        spec.height,
                        img_height,
                    )
                })?
        }
    };

    if abs_x >= img_width || abs_y >= img_height {
        return Err(anyhow!(
            "Capture '{}' origin ({}, {}) is outside image bounds {}x{}",
            spec.name,
            abs_x,
            abs_y,
            img_width,
            img_height,
        ));
    }

    let max_w = img_width - abs_x;
    let max_h = img_height - abs_y;

    if spec.width > max_w || spec.height > max_h {
        return Err(anyhow!(
            "Capture '{}' rectangle ({}, {}, {}x{}) exceeds image bounds {}x{}",
            spec.name,
            abs_x,
            abs_y,
            spec.width,
            spec.height,
            img_width,
            img_height,
        ));
    }

    Ok((abs_x, abs_y))
}

/// Validate capture specifications against an image without processing.
/// Opens the image, checks dimensions, and validates all capture specs.
fn validate_image(path: &Path, origin: Origin, specs: &[CaptureSpec]) -> Result<()> {
    let img =
        image::open(path).with_context(|| format!("Unable to open image '{}'", path.display()))?;
    let (img_width, img_height) = img.dimensions();

    eprintln!(
        "Validating {} ({}x{})",
        path.display(),
        img_width,
        img_height
    );

    for spec in specs {
        convert_coordinates(spec, origin, img_width, img_height).with_context(|| {
            format!(
                "Invalid capture spec '{}' for image '{}'",
                spec.name,
                path.display()
            )
        })?;

        let out_path = make_output_path(path, &spec.name)?;
        eprintln!("  '{}' -> {}", spec.name, out_path.display());
    }

    Ok(())
}

/// Process a single image file:
/// - Open the image
/// - For each capture spec, compute absolute coordinates based on origin
/// - Crop and save as <basename>_<spec.name>.<ext>
fn process_image(path: &Path, origin: Origin, specs: &[CaptureSpec], verbose: bool) -> Result<()> {
    let start = Instant::now();
    let img =
        image::open(path).with_context(|| format!("Unable to open image '{}'", path.display()))?;
    let decode_ms = start.elapsed().as_millis();

    let (img_width, img_height) = img.dimensions();

    let crop_start = Instant::now();

    for spec in specs {
        let (abs_x, abs_y) = convert_coordinates(spec, origin, img_width, img_height)
            .with_context(|| format!("Processing image '{}'", path.display()))?;

        let out_path = make_output_path(path, &spec.name)?;

        // Crop and save using the most native representation we can.
        crop_and_save(&img, abs_x, abs_y, spec.width, spec.height, &out_path)?;
    }

    if verbose {
        let crop_ms = crop_start.elapsed().as_millis();
        eprintln!(
            "Processed {} (decode: {} ms, crop+save: {} ms)",
            path.display(),
            decode_ms,
            crop_ms
        );
    }

    Ok(())
}

/// Crop and save using a representation close to the original image.
fn crop_and_save(
    img: &DynamicImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    out_path: &Path,
) -> Result<()> {
    img.crop_imm(x, y, width, height)
        .save(out_path)
        .with_context(|| format!("Unable to save image to '{}'", out_path.display()))?;
    Ok(())
}

/// Build output filename: <basename>_<`segment_name`>.<ext>
fn make_output_path(input: &Path, segment_name: &str) -> Result<PathBuf> {
    let file_name = input
        .file_name()
        .ok_or_else(|| anyhow!("Input path '{}' has no file name", input.display()))?
        .to_string_lossy();

    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem.to_string(), ext),
        _ => (file_name.to_string(), "png"), // default to png if no extension
    };

    let new_file_name = format!("{stem}_{segment_name}.{ext}");
    Ok(input.with_file_name(new_file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_capture_spec_valid() {
        let spec = parse_capture_spec("left:200x300:1200x1850").unwrap();
        assert_eq!(spec.name, "left");
        assert_eq!(spec.x, 200);
        assert_eq!(spec.y, 300);
        assert_eq!(spec.width, 1200);
        assert_eq!(spec.height, 1850);
    }

    #[test]
    fn test_parse_capture_spec_zero_coordinates() {
        let spec = parse_capture_spec("top:0x0:100x100").unwrap();
        assert_eq!(spec.name, "top");
        assert_eq!(spec.x, 0);
        assert_eq!(spec.y, 0);
        assert_eq!(spec.width, 100);
        assert_eq!(spec.height, 100);
    }

    #[test]
    fn test_parse_capture_spec_missing_parts() {
        let result = parse_capture_spec("left:200x300");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected format: <name>:<x>x<y>:<width>x<height>"));
    }

    #[test]
    fn test_parse_capture_spec_too_many_parts() {
        let result = parse_capture_spec("left:200x300:1200x1850:extra");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_capture_spec_invalid_number() {
        let result = parse_capture_spec("left:abcx300:1200x1850");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_capture_spec_zero_width() {
        let result = parse_capture_spec("left:200x300:0x1850");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Width and height must be positive"));
    }

    #[test]
    fn test_parse_capture_spec_zero_height() {
        let result = parse_capture_spec("left:200x300:1200x0");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Width and height must be positive"));
    }

    #[test]
    fn test_parse_pair_valid() {
        let result = parse_pair("100x200", 'x', "test", "spec").unwrap();
        assert_eq!(result, (100, 200));
    }

    #[test]
    fn test_parse_pair_missing_second() {
        let result = parse_pair("100", 'x', "test", "spec");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing second"));
    }

    #[test]
    fn test_parse_pair_too_many_parts() {
        let result = parse_pair("100x200x300", 'x', "test", "spec");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Too many components"));
    }

    #[test]
    fn test_parse_pair_invalid_first_number() {
        let result = parse_pair("abcx200", 'x', "test", "spec");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse first"));
    }

    #[test]
    fn test_parse_pair_invalid_second_number() {
        let result = parse_pair("100xabc", 'x', "test", "spec");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse second"));
    }

    #[test]
    fn test_origin_from_str_top_left_variants() {
        assert_eq!("tl".parse::<Origin>().unwrap(), Origin::TopLeft);
        assert_eq!("top-left".parse::<Origin>().unwrap(), Origin::TopLeft);
        assert_eq!("top_left".parse::<Origin>().unwrap(), Origin::TopLeft);
        assert_eq!("TL".parse::<Origin>().unwrap(), Origin::TopLeft);
        assert_eq!("Top-Left".parse::<Origin>().unwrap(), Origin::TopLeft);
    }

    #[test]
    fn test_origin_from_str_bottom_left_variants() {
        assert_eq!("bl".parse::<Origin>().unwrap(), Origin::BottomLeft);
        assert_eq!("bottom-left".parse::<Origin>().unwrap(), Origin::BottomLeft);
        assert_eq!("bottom_left".parse::<Origin>().unwrap(), Origin::BottomLeft);
        assert_eq!("BL".parse::<Origin>().unwrap(), Origin::BottomLeft);
        assert_eq!("Bottom-Left".parse::<Origin>().unwrap(), Origin::BottomLeft);
    }

    #[test]
    fn test_origin_from_str_invalid() {
        let result = "invalid".parse::<Origin>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid origin"));
    }

    #[test]
    fn test_make_output_path_with_extension() {
        let input = PathBuf::from("/path/to/image.jpg");
        let output = make_output_path(&input, "left").unwrap();
        assert_eq!(output, PathBuf::from("/path/to/image_left.jpg"));
    }

    #[test]
    fn test_make_output_path_with_multiple_dots() {
        let input = PathBuf::from("/path/to/my.image.file.png");
        let output = make_output_path(&input, "crop").unwrap();
        assert_eq!(output, PathBuf::from("/path/to/my.image.file_crop.png"));
    }

    #[test]
    fn test_make_output_path_no_extension() {
        let input = PathBuf::from("/path/to/image");
        let output = make_output_path(&input, "output").unwrap();
        assert_eq!(output, PathBuf::from("/path/to/image_output.png"));
    }

    #[test]
    fn test_make_output_path_different_extensions() {
        let extensions = vec!["jpg", "png", "gif", "bmp", "tiff", "webp"];
        for ext in extensions {
            let input = PathBuf::from(format!("/path/to/image.{ext}"));
            let output = make_output_path(&input, "test").unwrap();
            assert_eq!(output, PathBuf::from(format!("/path/to/image_test.{ext}")));
        }
    }

    #[test]
    fn test_make_output_path_special_characters_in_name() {
        let input = PathBuf::from("/path/to/image-with-dashes.jpg");
        let output = make_output_path(&input, "segment_name").unwrap();
        assert_eq!(
            output,
            PathBuf::from("/path/to/image-with-dashes_segment_name.jpg")
        );
    }

    #[test]
    fn test_convert_coordinates_top_left_origin() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 100,
            y: 200,
            width: 50,
            height: 75,
        };
        let (abs_x, abs_y) = convert_coordinates(&spec, Origin::TopLeft, 1000, 1000).unwrap();
        assert_eq!(abs_x, 100);
        assert_eq!(abs_y, 200);
    }

    #[test]
    fn test_convert_coordinates_bottom_left_origin() {
        // For a 1000px tall image, capturing from bottom-left (0, 0) with height 100
        // should convert to top-left (0, 900)
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let (abs_x, abs_y) = convert_coordinates(&spec, Origin::BottomLeft, 1000, 1000).unwrap();
        assert_eq!(abs_x, 0);
        assert_eq!(abs_y, 900);
    }

    #[test]
    fn test_convert_coordinates_bottom_left_origin_middle() {
        // For a 1000px tall image, capturing from bottom-left (50, 200) with height 100
        // should convert to top-left (50, 700)
        // Formula: abs_y = img_height - spec.y - spec.height = 1000 - 200 - 100 = 700
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 50,
            y: 200,
            width: 100,
            height: 100,
        };
        let (abs_x, abs_y) = convert_coordinates(&spec, Origin::BottomLeft, 1000, 1000).unwrap();
        assert_eq!(abs_x, 50);
        assert_eq!(abs_y, 700);
    }

    #[test]
    fn test_convert_coordinates_top_left_at_edge() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 900,
            y: 900,
            width: 100,
            height: 100,
        };
        let (abs_x, abs_y) = convert_coordinates(&spec, Origin::TopLeft, 1000, 1000).unwrap();
        assert_eq!(abs_x, 900);
        assert_eq!(abs_y, 900);
    }

    #[test]
    fn test_convert_coordinates_bottom_left_at_top() {
        // Capturing from the very top of the image in bottom-left coordinates
        // For a 1000px tall image, y=900 height=100 should give abs_y=0
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 0,
            y: 900,
            width: 100,
            height: 100,
        };
        let (abs_x, abs_y) = convert_coordinates(&spec, Origin::BottomLeft, 1000, 1000).unwrap();
        assert_eq!(abs_x, 0);
        assert_eq!(abs_y, 0);
    }

    #[test]
    fn test_convert_coordinates_x_out_of_bounds() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 1000,
            y: 0,
            width: 100,
            height: 100,
        };
        let result = convert_coordinates(&spec, Origin::TopLeft, 1000, 1000);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("is outside image bounds"));
    }

    #[test]
    fn test_convert_coordinates_y_out_of_bounds_top_left() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 0,
            y: 1000,
            width: 100,
            height: 100,
        };
        let result = convert_coordinates(&spec, Origin::TopLeft, 1000, 1000);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("is outside image bounds"));
    }

    #[test]
    fn test_convert_coordinates_y_out_of_bounds_bottom_left() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 0,
            y: 1001,
            width: 100,
            height: 100,
        };
        let result = convert_coordinates(&spec, Origin::BottomLeft, 1000, 1000);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("is outside image height"));
    }

    #[test]
    fn test_convert_coordinates_width_exceeds_bounds() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 900,
            y: 0,
            width: 200,
            height: 100,
        };
        let result = convert_coordinates(&spec, Origin::TopLeft, 1000, 1000);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeds image bounds"));
    }

    #[test]
    fn test_convert_coordinates_height_exceeds_bounds() {
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 0,
            y: 900,
            width: 100,
            height: 200,
        };
        let result = convert_coordinates(&spec, Origin::TopLeft, 1000, 1000);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeds image bounds"));
    }

    #[test]
    fn test_convert_coordinates_bottom_left_overflow() {
        // When y + height > img_height in bottom-left coordinates
        let spec = CaptureSpec {
            name: "test".to_string(),
            x: 0,
            y: 950,
            width: 100,
            height: 100,
        };
        let result = convert_coordinates(&spec, Origin::BottomLeft, 1000, 1000);
        assert!(result.is_err());
    }
}
