//! Guards the image codecs the client needs to display a Lemmy feed.
//!
//! This test exists because the failure it prevents is invisible. `egui_extras`
//! decodes images through the `image` crate but enables no formats of its own,
//! and `eframe`/`arboard` only enable `png`. A missing codec is neither a
//! compile error nor a runtime error — egui simply draws nothing, with no
//! message anywhere — so "some pictures are not rendered" has no other symptom
//! and nothing else in the suite notices.

use image::ImageFormat;

// Real encoded bytes, 2x2 pixels, produced once by an image encoder and
// embedded here. Decoding them exercises the codec end to end rather than
// trusting a feature flag, and needs no binary fixture and no encoder.

/// 81 bytes.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAGElEQVR4nAXBAQEAAAjDIG7/zhNE0k3CAz7tBf5/xlWuAAAAAElFTkSuQmCC";

/// 694 bytes.
const JPEG: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBT/wAARCAACAAIDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD6v+APhXRb/wCBHw4ubnR7C4uZvDemySzS2qM8jm1jJZiRkkkkkmiiiv8AP3Ov+Rniv+vk/wD0pn45mP8Artb/ABS/Nn//2Q==";

/// 78 bytes.
const WEBP: &str = "UklGRkYAAABXRUJQVlA4IDoAAADQAQCdASoCAAIAAUAmJZgCdAEO+KkLAAD+68fy0Z8T/GL/+rQ3mHY2QAs0MmFD/4tD+3n/BrbSbQAA";

/// 46 bytes.
const GIF: &str = "R0lGODdhAgACAIEAAAAA/wD/AP8AAP//ACwAAAAAAgACAAAIBwAFBAAwICAAOw==";

/// A real AVIF captured from a live federated post (`lemmy.zip`, reached through
/// `lemmy.world`): 4,623 bytes, 440x536. Real bytes on purpose — the pure-Rust
/// decoder that was tried first failed on exactly this file with "AV1 tile 0
/// entropy validation failed", and a generated test image would not have caught
/// that.
const REAL_AVIF: &[u8] = include_bytes!("../fixtures/pictures/avif-post.avif");

/// AVIF is the one format Lemmy serves that the `image` crate cannot decode in
/// this build. Its only AVIF decoder is behind `avif-native`, which links the C
/// library libdav1d and so turns a system package into a build requirement
/// (measured: `dav1d-sys` fails without `dav1d.pc`). `src/avif.rs` decodes AVIF
/// in pure Rust instead, through a loader of its own.
#[test]
fn avif_is_decoded_by_the_clients_own_loader_not_by_the_image_crate() {
    assert_eq!(
        ImageFormat::from_extension("avif"),
        Some(ImageFormat::Avif),
        "the image crate knows the extension, even where it cannot decode it"
    );
    assert!(
        !ImageFormat::Avif.reading_enabled(),
        "the image crate decodes AVIF in this build, so the loader in src/avif.rs is \
         redundant — and turning on `avif-native` would have made a system library a \
         build requirement"
    );

    // What instances actually serve, decoded the way the application decodes it.
    let image = limonada::avif::decode(REAL_AVIF).expect("the captured AVIF should decode");
    assert_eq!(image.size, [440, 536]);
}

/// The formats a Lemmy instance realistically serves, with real bytes for each.
fn required() -> [(&'static str, ImageFormat, &'static str); 4] {
    [
        ("png", ImageFormat::Png, PNG),
        ("jpeg", ImageFormat::Jpeg, JPEG),
        ("webp", ImageFormat::WebP, WEBP),
        ("gif", ImageFormat::Gif, GIF),
    ]
}

/// The exact predicate `egui_extras`' loader consults before it will attempt a
/// URI: `ImageFormat::from_extension(ext).is_some_and(|f| f.reading_enabled())`.
/// If this is false, the image is never even fetched.
#[test]
fn egui_extras_would_accept_every_format_lemmy_serves() {
    for (name, format, _) in required() {
        assert!(
            format.reading_enabled(),
            "the {name} decoder is not compiled in, so egui_extras renders every \
             {name} image as blank space. Enable \"{name}\" on the `image` \
             dependency in Cargo.toml."
        );
    }
}

/// `reading_enabled` could in principle report a codec that then fails, so real
/// bytes are decoded too.
#[test]
fn real_encoded_bytes_decode_to_pixels() {
    for (name, format, encoded) in required() {
        let bytes = decode_base64(encoded);
        let image = image::load_from_memory_with_format(&bytes, format)
            .unwrap_or_else(|e| panic!("{name} bytes must decode, not merely be enabled: {e}"));
        assert_eq!((image.width(), image.height()), (2, 2), "{name} dimensions");
    }
}

/// A thumbnail URL is judged by its extension before its bytes are fetched, so
/// a URL whose extension maps to a disabled codec is rejected outright — which
/// is what made JPEG thumbnails disappear while PNG ones worked.
#[test]
fn typical_lemmy_thumbnail_urls_resolve_to_an_enabled_codec() {
    for uri in [
        "https://lemmy.ml/pictrs/image/abc.jpeg",
        "https://lemmy.ml/pictrs/image/abc.jpg",
        "https://lemmy.ml/pictrs/image/abc.png",
        "https://lemmy.ml/pictrs/image/abc.webp",
        "https://lemmy.ml/pictrs/image/abc.gif",
    ] {
        let ext = std::path::Path::new(uri)
            .extension()
            .and_then(|ext| ext.to_str())
            .expect("a pictrs image URL should carry an extension");
        let format = ImageFormat::from_extension(ext)
            .unwrap_or_else(|| panic!("the image crate does not know .{ext}"));
        assert!(
            format.reading_enabled(),
            "egui_extras would refuse {uri}: .{ext} is not an enabled format"
        );
    }
}

/// Minimal base64 decoder, so this test adds no dependency of its own. Padding
/// and whitespace are skipped.
fn decode_base64(input: &str) -> Vec<u8> {
    fn sextet(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let Some(value) = sextet(byte) else {
            continue;
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// The decoder is load-bearing for the tests above, so it gets checked too.
#[test]
fn the_base64_helper_decodes_the_expected_bytes() {
    assert_eq!(decode_base64("TWFu"), b"Man");
    assert_eq!(decode_base64("TWE="), b"Ma");
    assert_eq!(decode_base64("TQ=="), b"M");
    assert_eq!(decode_base64(""), b"");
    // Cross-check: the PNG blob must start with the PNG signature.
    assert_eq!(&decode_base64(PNG)[..8], b"\x89PNG\r\n\x1a\n");
    // ...and the JPEG blob with SOI, and the GIF blob with its header.
    assert_eq!(&decode_base64(JPEG)[..3], b"\xff\xd8\xff");
    assert_eq!(&decode_base64(GIF)[..6], b"GIF87a");
    assert_eq!(&decode_base64(WEBP)[..4], b"RIFF");
}
