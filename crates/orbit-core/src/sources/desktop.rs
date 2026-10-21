//! SIM-56: the whole screen as Deimos sees it. The engine captures the
//! primary display in true screen pixels with no downscale; a capture can
//! carry at most `MAX_WINDOW_WIDTH`×`MAX_WINDOW_HEIGHT` and `MAX_SOURCE_BYTES`.
//! This fits one into the other and keeps the scale between them, so a
//! desktop action Deimos places on the image lands on the screen under it.
//! Pure: bytes in, bytes out; the file and the engine are the daemon's.

use crate::refusal::{Expected, Parameter, Refusal};

/// The largest display macOS drives at native pixels: 8K, 7680×4320, which
/// Apple supports on its Mac Studio and Mac Pro. Apple's own largest display,
/// the Pro Display XDR, is 6016×3384, and Mars's MacBook is 3024×1964. A
/// capture with more pixels than this is not of a Mac display and is refused.
pub const MAX_DESKTOP_WIDTH: u32 = 7680;
pub const MAX_DESKTOP_HEIGHT: u32 = 4320;

/// The largest PNG the engine can write for that display, read whole from its
/// file: the case where PNG compresses nothing. It protects the daemon's
/// memory from one file. Derived, not measured: 8-bit RGBA (what a screen
/// capture is), one filter byte per row, DEFLATE stored blocks of 65 535 bytes
/// with 5 bytes each, the zlib header and checksum, the signature and IHDR,
/// a 12-byte header per IDAT chunk of at least 8 KiB, and 64 KiB for the
/// colour profile and other ancillary chunks a screenshot carries. That is
/// 132 984 861 bytes. A real capture is far smaller, since screens compress.
pub const MAX_DESKTOP_PNG_BYTES: usize =
    worst_case_png_bytes(MAX_DESKTOP_WIDTH, MAX_DESKTOP_HEIGHT);

const fn worst_case_png_bytes(width: u32, height: u32) -> usize {
    let raw = height as usize * (1 + width as usize * 4);
    let deflate = raw + 5 * raw.div_ceil(65_535) + 2 + 4;
    let idat_headers = (deflate.div_ceil(8 * 1024)) * 12;
    8 + 25 + idat_headers + deflate + 12 + 64 * 1024
}

/// Each time a fitted capture is still over `MAX_SOURCE_BYTES`, its box
/// shrinks to three quarters, which takes about 44 % off its pixels.
const STEP_NUMERATOR: u32 = 3;
const STEP_DENOMINATOR: u32 = 4;
/// No narrower than this: a quarter of the window ceiling. Below it a desktop
/// shows nothing legible, so a screen that still does not fit is refused with
/// its numbers instead of being delivered as a smudge.
const MIN_FITTED_WIDTH: u32 = super::MAX_WINDOW_WIDTH / 4;

/// How the delivered image relates to the screen: what Deimos saw, and the
/// true pixels it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopScale {
    pub screen_width: u32,
    pub screen_height: u32,
    pub image_width: u32,
    pub image_height: u32,
}

impl DesktopScale {
    /// What Deimos is told with the image, in plain words.
    pub fn describe(&self) -> String {
        format!(
            "Esta imagen mide {} × {} píxeles y muestra la pantalla entera, que mide {} × {}. Las coordenadas de una acción de escritorio se dan sobre esta imagen, de 0 a {} en horizontal y de 0 a {} en vertical; Orbit las lleva a la pantalla.",
            self.image_width,
            self.image_height,
            self.screen_width,
            self.screen_height,
            self.image_width - 1,
            self.image_height - 1,
        )
    }

    /// The screen pixel under an image coordinate on one axis: the centre of
    /// the screen pixels that image pixel averages.
    fn axis(
        value: &serde_json::Value,
        image: u32,
        screen: u32,
        name: &str,
        image_width: u32,
        image_height: u32,
    ) -> Result<serde_json::Value, Refusal> {
        let refusal = |upper_inclusive| Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::DesktopCoordinate {
                axis: name.into(),
                received: value.as_number().cloned(),
                image,
                screen,
                image_width,
                image_height,
                upper_inclusive,
            },
        };
        if let Some(pixel) = value.as_u64() {
            if pixel >= u64::from(image) {
                return Err(refusal(false));
            }
            let mapped = (2 * pixel + 1) * u64::from(screen) / (2 * u64::from(image));
            return Ok(serde_json::json!(mapped));
        }
        if let Some(position) = value.as_f64() {
            if !(0.0..=f64::from(image)).contains(&position) {
                return Err(refusal(true));
            }
            return Ok(serde_json::json!(
                position * f64::from(screen) / f64::from(image)
            ));
        }
        Err(refusal(false))
    }
}

/// The coordinate arguments of a desktop action, by axis.
const HORIZONTAL: [&str; 3] = ["x", "from_x", "to_x"];
const VERTICAL: [&str; 3] = ["y", "from_y", "to_y"];

/// Whether these engine arguments act on the desktop frame:
/// `target: {kind: "desktop"}`, or the legacy `scope: "desktop"`.
pub fn is_desktop_action(arguments: &serde_json::Value) -> bool {
    arguments["target"]["kind"] == "desktop" || arguments["scope"] == "desktop"
}

/// Desktop-action arguments with their coordinates taken from the delivered
/// image to the screen. `None` when the action is not on the desktop, which
/// leaves it as it was. Refused, by name and with the numbers, when no
/// desktop capture was delivered (the coordinates would refer to nothing),
/// or a coordinate is off the image.
pub fn map_desktop_action(
    arguments: &serde_json::Value,
    scale: Option<&DesktopScale>,
) -> Result<Option<serde_json::Value>, Refusal> {
    if !is_desktop_action(arguments) {
        return Ok(None);
    }
    let Some(scale) = scale else {
        return Err(Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::DesktopFrame,
        });
    };
    let mut mapped = arguments.clone();
    for (names, image, screen) in [
        (HORIZONTAL, scale.image_width, scale.screen_width),
        (VERTICAL, scale.image_height, scale.screen_height),
    ] {
        for name in names {
            if let Some(value) = arguments.get(name) {
                mapped[name] = DesktopScale::axis(
                    value,
                    image,
                    screen,
                    name,
                    scale.image_width,
                    scale.image_height,
                )?;
            }
        }
    }
    Ok(Some(mapped))
}

/// A full screen fitted for delivery, with its scale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittedDesktop {
    pub png: Vec<u8>,
    pub scale: DesktopScale,
}

/// Why a full screen could not be fitted, with its numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopFitError {
    #[error("{self:?}")]
    FileTooLarge { bytes: usize, max_bytes: usize },
    #[error("{self:?}")]
    DisplayTooLarge {
        width: u32,
        height: u32,
        max_width: u32,
        max_height: u32,
    },
    #[error("{self:?}")]
    Interlaced,
    #[error("{self:?}")]
    Unreadable(String),
    #[error("{self:?}")]
    CannotFit {
        width: u32,
        height: u32,
        bytes: usize,
        max_bytes: usize,
    },
}

/// The screen in `png`, area-averaged into the largest size that fits
/// `MAX_WINDOW_WIDTH`×`MAX_WINDOW_HEIGHT` with its proportions, re-encoded,
/// and stepped down until the PNG fits `MAX_SOURCE_BYTES`. The source is
/// decoded row by row, so its pixels are never all in memory.
pub fn fit_desktop_png(png: &[u8]) -> Result<FittedDesktop, DesktopFitError> {
    if png.len() > MAX_DESKTOP_PNG_BYTES {
        return Err(DesktopFitError::FileTooLarge {
            bytes: png.len(),
            max_bytes: MAX_DESKTOP_PNG_BYTES,
        });
    }
    let (width, height) = header(png)?;
    if width > MAX_DESKTOP_WIDTH || height > MAX_DESKTOP_HEIGHT {
        return Err(DesktopFitError::DisplayTooLarge {
            width,
            height,
            max_width: MAX_DESKTOP_WIDTH,
            max_height: MAX_DESKTOP_HEIGHT,
        });
    }
    let mut bounds = (super::MAX_WINDOW_WIDTH, super::MAX_WINDOW_HEIGHT);
    loop {
        let (image_width, image_height) = fitted_size(width, height, bounds);
        let pixels = downscale(png, width, height, image_width, image_height)?;
        let encoded = encode(&pixels, image_width, image_height)?;
        if encoded.len() <= super::MAX_SOURCE_BYTES {
            return Ok(FittedDesktop {
                png: encoded,
                scale: DesktopScale {
                    screen_width: width,
                    screen_height: height,
                    image_width,
                    image_height,
                },
            });
        }
        let next = (
            bounds.0 * STEP_NUMERATOR / STEP_DENOMINATOR,
            bounds.1 * STEP_NUMERATOR / STEP_DENOMINATOR,
        );
        if fitted_size(width, height, next).0 < MIN_FITTED_WIDTH.min(width) {
            return Err(DesktopFitError::CannotFit {
                width: image_width,
                height: image_height,
                bytes: encoded.len(),
                max_bytes: super::MAX_SOURCE_BYTES,
            });
        }
        bounds = next;
    }
}

fn decoder(png: &[u8]) -> png::Decoder<std::io::Cursor<&[u8]>> {
    // Rows only: the limit is for one decoded row and the PNG's own chunks.
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(png),
        png::Limits {
            bytes: 16 * 1024 * 1024,
        },
    );
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder
}

fn header(png: &[u8]) -> Result<(u32, u32), DesktopFitError> {
    let mut decoder = decoder(png);
    let info = decoder
        .read_header_info()
        .map_err(|error| DesktopFitError::Unreadable(error.to_string()))?;
    if info.width == 0 || info.height == 0 {
        return Err(DesktopFitError::Unreadable("mide 0 píxeles".into()));
    }
    Ok((info.width, info.height))
}

/// The largest size inside `bounds` with the screen's proportions.
fn fitted_size(width: u32, height: u32, bounds: (u32, u32)) -> (u32, u32) {
    if width <= bounds.0 && height <= bounds.1 {
        return (width, height);
    }
    let (width, height) = (u64::from(width), u64::from(height));
    let (max_width, max_height) = (u64::from(bounds.0), u64::from(bounds.1));
    let (fitted_width, fitted_height) = if width * max_height >= height * max_width {
        (max_width, height * max_width / width)
    } else {
        (width * max_height / height, max_height)
    };
    (
        u32::try_from(fitted_width.max(1)).unwrap_or(u32::MAX),
        u32::try_from(fitted_height.max(1)).unwrap_or(u32::MAX),
    )
}

/// Area average: every screen pixel adds to the one image pixel it falls in,
/// and each image pixel is the mean of its own. RGB out; a screen is opaque.
fn downscale(
    png: &[u8],
    width: u32,
    height: u32,
    image_width: u32,
    image_height: u32,
) -> Result<Vec<u8>, DesktopFitError> {
    let unreadable = |error: png::DecodingError| DesktopFitError::Unreadable(error.to_string());
    let mut reader = decoder(png).read_info().map_err(unreadable)?;
    if reader.info().interlaced {
        return Err(DesktopFitError::Interlaced);
    }
    let (color, _) = reader.output_color_type();
    let channels = color.samples();
    let (width, height) = (width as usize, height as usize);
    let (image_width, image_height) = (image_width as usize, image_height as usize);
    let column: Vec<usize> = (0..width).map(|x| x * image_width / width).collect();
    let mut sums = vec![0_u64; image_width * 3];
    let mut counts = vec![0_u64; image_width];
    let mut out = Vec::with_capacity(image_width * image_height * 3);
    let mut current = 0_usize;
    for y in 0..height {
        let row = reader
            .next_row()
            .map_err(unreadable)?
            .ok_or_else(|| DesktopFitError::Unreadable(format!("le faltan filas desde la {y}")))?;
        let data = row.data();
        if data.len() < width * channels {
            return Err(DesktopFitError::Unreadable(format!(
                "la fila {y} está incompleta"
            )));
        }
        let target = y * image_height / height;
        if target != current {
            flush_row(&mut sums, &mut counts, &mut out);
            current = target;
        }
        for (x, &pixel) in column.iter().enumerate() {
            let source = &data[x * channels..x * channels + channels];
            let (red, green, blue) = match channels {
                1 | 2 => (source[0], source[0], source[0]),
                _ => (source[0], source[1], source[2]),
            };
            sums[pixel * 3] += u64::from(red);
            sums[pixel * 3 + 1] += u64::from(green);
            sums[pixel * 3 + 2] += u64::from(blue);
            counts[pixel] += 1;
        }
    }
    flush_row(&mut sums, &mut counts, &mut out);
    Ok(out)
}

/// One finished image row: each pixel's mean, and the sums cleared.
fn flush_row(sums: &mut [u64], counts: &mut [u64], out: &mut Vec<u8>) {
    for (pixel, count) in counts.iter_mut().enumerate() {
        for sum in &mut sums[pixel * 3..pixel * 3 + 3] {
            out.push(u8::try_from(*sum / (*count).max(1)).unwrap_or(u8::MAX));
            *sum = 0;
        }
        *count = 0;
    }
}

fn encode(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DesktopFitError> {
    let failed = |error: png::EncodingError| DesktopFitError::Unreadable(error.to_string());
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder.write_header().map_err(failed)?;
        writer.write_image_data(pixels).map_err(failed)?;
        writer.finish().map_err(failed)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
