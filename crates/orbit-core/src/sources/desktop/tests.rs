#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::json;

fn png_of(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&pixel(x, y));
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    bytes
}

fn bands(x: u32, y: u32) -> [u8; 4] {
    [(x / 64 % 256) as u8, (y / 64 % 256) as u8, 40, 255]
}

#[test]
fn the_ceiling_is_the_worst_case_png_of_an_8k_display() {
    assert_eq!(MAX_DESKTOP_PNG_BYTES, 132_984_861);
    const { assert!(MAX_DESKTOP_PNG_BYTES > 7680 * 4320 * 4) };
}

#[test]
fn a_retina_screen_is_fitted_to_a_capture_with_its_proportions_and_scale() {
    let fitted = fit_desktop_png(&png_of(3024, 1964, bands)).unwrap();
    assert_eq!(
        fitted.scale,
        DesktopScale {
            screen_width: 3024,
            screen_height: 1964,
            image_width: 1280,
            image_height: 831,
        }
    );
    assert!(fitted.png.len() <= crate::sources::MAX_SOURCE_BYTES);
    crate::sources::validate_png(&fitted.png, 1280, 831).unwrap();
}

#[test]
fn the_average_keeps_the_colour_of_a_uniform_screen() {
    let fitted = fit_desktop_png(&png_of(2560, 1600, |_, _| [10, 200, 30, 255])).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(&fitted.png));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    assert!(pixels.chunks(3).all(|pixel| pixel == [10, 200, 30]));
}

#[test]
fn a_screen_that_compresses_badly_steps_down_until_it_fits() {
    // Noise does not compress: 1280×831 of it is 3.2 MB.
    let mut state = 0x2545_f491_u32;
    let mut noise = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let pixels: Vec<[u8; 4]> = (0..3024 * 1964)
        .map(|_| {
            let value = noise().to_le_bytes();
            [value[0], value[1], value[2], 255]
        })
        .collect();
    let png = png_of(3024, 1964, |x, y| pixels[(y * 3024 + x) as usize]);
    let fitted = fit_desktop_png(&png).unwrap();
    assert!(fitted.png.len() <= crate::sources::MAX_SOURCE_BYTES);
    assert!(fitted.scale.image_width < 1280, "{:?}", fitted.scale);
    assert!(fitted.scale.image_width >= MIN_FITTED_WIDTH);
}

#[test]
fn a_screen_larger_than_any_mac_display_is_refused_with_its_numbers() {
    let error = fit_desktop_png(&png_of(7681, 2, bands)).unwrap_err();
    assert_eq!(
        error,
        DesktopFitError::DisplayTooLarge {
            width: 7681,
            height: 2,
            max_width: 7680,
            max_height: 4320,
        }
    );
    assert!(error.to_string().contains("7681"));
}

#[test]
fn a_screen_that_already_fits_is_kept_at_its_size() {
    let fitted = fit_desktop_png(&png_of(1024, 768, bands)).unwrap();
    assert_eq!(
        (fitted.scale.image_width, fitted.scale.image_height),
        (1024, 768)
    );
}

const SCALE: DesktopScale = DesktopScale {
    screen_width: 3024,
    screen_height: 1964,
    image_width: 1280,
    image_height: 831,
};

#[test]
fn a_desktop_coordinate_lands_on_the_centre_of_the_screen_pixels_it_averages() {
    let click = json!({"target": {"kind": "desktop", "display_id": "primary"}, "x": 640, "y": 415});
    let mapped = map_desktop_action(&click, Some(&SCALE)).unwrap().unwrap();
    assert_eq!(mapped["x"], 1513);
    assert_eq!(mapped["y"], 982);
    assert_eq!(mapped["target"], click["target"]);
    let corner = json!({"target": {"kind": "desktop"}, "x": 0, "y": 830});
    let mapped = map_desktop_action(&corner, Some(&SCALE)).unwrap().unwrap();
    assert_eq!(
        (mapped["x"].as_u64(), mapped["y"].as_u64()),
        (Some(1), Some(1962))
    );
    let drag = json!({"scope": "desktop", "from_x": 1279, "from_y": 0, "to_x": 0.5, "to_y": 831.0});
    let mapped = map_desktop_action(&drag, Some(&SCALE)).unwrap().unwrap();
    assert_eq!(mapped["from_x"], 3022);
    assert_eq!(mapped["to_y"], 1964.0);
}

#[test]
fn a_window_action_is_left_as_it_came() {
    let click = json!({"pid": 11, "window_id": 22, "x": 5000, "y": 5000});
    assert_eq!(map_desktop_action(&click, None).unwrap(), None);
}

#[test]
fn a_desktop_action_without_a_capture_or_off_the_image_is_refused_by_name() {
    let click = json!({"target": {"kind": "desktop"}, "x": 10, "y": 10});
    let none = map_desktop_action(&click, None).unwrap_err();
    assert_eq!(
        serde_json::to_value(none).unwrap()["expected"]["kind"],
        "desktop_frame"
    );
    let outside = json!({"target": {"kind": "desktop"}, "x": 1280, "y": 10});
    let error = map_desktop_action(&outside, Some(&SCALE)).unwrap_err();
    let expected = serde_json::to_value(error).unwrap()["expected"].clone();
    assert_eq!(expected["kind"], "desktop_coordinate");
    assert_eq!(expected["axis"], "x");
    assert_eq!(expected["received"], 1280);
    assert_eq!(expected["image_width"], 1280);
    assert_eq!(expected["image_height"], 831);
    assert_eq!(expected["upper_inclusive"], false);
    let text = json!({"target": {"kind": "desktop"}, "x": "centro"});
    assert!(map_desktop_action(&text, Some(&SCALE)).is_err());
}

#[test]
fn deimos_is_told_the_two_sizes_in_plain_words() {
    let text = SCALE.describe();
    for number in ["1280", "831", "3024", "1964", "1279", "830"] {
        assert!(text.contains(number), "{text}");
    }
}
