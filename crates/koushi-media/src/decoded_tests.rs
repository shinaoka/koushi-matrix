use super::decoded::{DecodeLimits, DecodedRgbaImage, DecodedRgbaImageError};

#[test]
fn premultiplied_pixels_become_straight_alpha() {
    // Opaque, half-transparent, and fully transparent pixels.
    let premultiplied = vec![200, 100, 50, 255, 64, 32, 0, 128, 9, 9, 9, 0];
    let image =
        DecodedRgbaImage::from_premultiplied_rgba(3, 1, 12, premultiplied, DecodeLimits::default())
            .expect("valid layout");
    assert_eq!(
        image.pixels(),
        &[200, 100, 50, 255, 128, 64, 0, 128, 0, 0, 0, 0]
    );
}

#[test]
fn padded_rows_are_compacted() {
    let mut padded = Vec::new();
    padded.extend_from_slice(&[1, 2, 3, 255, 0xAA, 0xAA]);
    padded.extend_from_slice(&[4, 5, 6, 255, 0xAA, 0xAA]);
    let image = DecodedRgbaImage::from_straight_rgba(1, 2, 6, padded, DecodeLimits::default())
        .expect("valid layout");
    assert_eq!(image.pixels(), &[1, 2, 3, 255, 4, 5, 6, 255]);
}

#[test]
fn layout_and_limits_are_checked_before_adoption() {
    let limits = DecodeLimits {
        max_dimension: 4,
        max_allocation_bytes: 32,
    };
    assert_eq!(
        DecodedRgbaImage::from_straight_rgba(5, 1, 20, vec![0; 20], limits),
        Err(DecodedRgbaImageError::TooLarge)
    );
    assert_eq!(
        DecodedRgbaImage::from_straight_rgba(3, 3, 12, vec![0; 36], limits),
        Err(DecodedRgbaImageError::TooLarge)
    );
    assert_eq!(
        DecodedRgbaImage::from_straight_rgba(0, 1, 0, Vec::new(), limits),
        Err(DecodedRgbaImageError::TooLarge)
    );
    assert_eq!(
        DecodedRgbaImage::from_straight_rgba(2, 2, 4, vec![0; 16], limits),
        Err(DecodedRgbaImageError::InvalidLayout)
    );
    assert_eq!(
        DecodedRgbaImage::from_straight_rgba(2, 2, 8, vec![0; 15], limits),
        Err(DecodedRgbaImageError::InvalidLayout)
    );
}

#[test]
fn default_limits_match_the_pure_decoder() {
    let limits = DecodeLimits::default();
    assert!(limits.admits(5712, 4284));
    assert!(!limits.admits(16_385, 1));
    assert!(!limits.admits(16_384, 16_384));
}
