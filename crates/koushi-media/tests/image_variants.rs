use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use koushi_media::{ImagePreparationPolicy, PreparedImageFormat, prepare_image_variants};

fn synthetic_png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 251) as u8, (y % 239) as u8, 127, ((x + y) % 256) as u8])
    });
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode fixture");
    bytes.into_inner()
}

fn synthetic_jpeg(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 251) as u8, (y % 239) as u8, 127, 255])
    });
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Jpeg)
        .expect("encode fixture");
    bytes.into_inner()
}

fn synthetic_apng() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(2, 0).expect("enable APNG");
        encoder.validate_sequence(true);
        let mut writer = encoder.write_header().expect("write APNG header");
        writer
            .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255])
            .expect("write APNG frame one");
        writer
            .write_image_data(&[0, 0, 255, 255, 255, 255, 0, 255])
            .expect("write APNG frame two");
        writer.finish().expect("finish APNG");
    }
    bytes
}

#[test]
fn png_offers_original_resized_png_and_alpha_preserving_webp() {
    let source = synthetic_png(96, 64);
    let variants = prepare_image_variants(
        &source,
        "sample.png",
        "image/png",
        &ImagePreparationPolicy {
            target_long_edge: 48,
            quality_percent: 82,
        },
    )
    .expect("prepare PNG");

    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.format)
            .collect::<Vec<_>>(),
        vec![
            PreparedImageFormat::Png,
            PreparedImageFormat::Png,
            PreparedImageFormat::WebP
        ]
    );
    assert_eq!(variants[1].mime_type, "image/png");
    assert_eq!(variants[1].dimensions, (48, 32));
    assert!(variants[1].metadata_stripped);
    assert_eq!(variants[2].mime_type, "image/webp");
    assert_eq!(
        image::load_from_memory(&variants[2].bytes)
            .unwrap()
            .color()
            .has_alpha(),
        true
    );
}

#[test]
fn jpeg_offers_real_jpeg_and_webp_outputs_and_recommends_no_larger_candidate() {
    let source = synthetic_jpeg(96, 64);
    let variants = prepare_image_variants(
        &source,
        "sample.jpg",
        "image/jpeg",
        &ImagePreparationPolicy {
            target_long_edge: 48,
            quality_percent: 75,
        },
    )
    .expect("prepare JPEG");

    assert_eq!(variants[0].format, PreparedImageFormat::Jpeg);
    assert_eq!(variants[1].mime_type, "image/jpeg");
    assert_eq!(variants[2].mime_type, "image/webp");
    let recommended = variants.iter().find(|variant| variant.recommended).unwrap();
    assert!(recommended.bytes.len() <= source.len());
}

#[test]
fn unsupported_or_animated_input_remains_original_only() {
    let gif = b"GIF89a synthetic animated fixture";
    let variants = prepare_image_variants(
        gif,
        "animation.gif",
        "image/gif",
        &ImagePreparationPolicy::default(),
    )
    .expect("original fallback");

    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].bytes, gif);
    assert_eq!(variants[0].mime_type, "image/gif");
    assert!(variants[0].recommended);
}

#[test]
fn animated_png_remains_original_only() {
    let apng = synthetic_apng();
    let variants = prepare_image_variants(
        &apng,
        "animation.png",
        "image/png",
        &ImagePreparationPolicy::default(),
    )
    .expect("retain APNG original");

    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].bytes, apng);
    assert_eq!(variants[0].mime_type, "image/png");
    assert!(variants[0].recommended);
}

#[test]
fn spoofed_image_declaration_uses_binary_mime_and_extension() {
    let source = b"not actually a png";
    let variants = prepare_image_variants(
        source,
        "spoofed.png",
        "image/png",
        &ImagePreparationPolicy::default(),
    )
    .expect("retain unknown original safely");

    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].mime_type, "application/octet-stream");
    assert_eq!(variants[0].filename, "spoofed.bin");
    assert_eq!(variants[0].format, PreparedImageFormat::Other);
}
