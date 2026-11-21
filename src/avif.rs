//! AVIF decoding, wired into egui's image loader stack.
//!
//! Instances serve AVIF and nothing in the default build could read it: `image`
//! has no AVIF decoder unless `avif-native` is enabled, and that feature links
//! the C library libdav1d, which makes a system package a build requirement (its
//! `dav1d-sys` build script fails without `dav1d.pc`). Losing a whole format to a
//! build dependency is a poor trade, so AVIF is decoded here with the pure-Rust
//! `zenavif` instead — and because `image` cannot decode it, egui needs a loader
//! of its own rather than a feature flag.
//!
//! The loader is installed *after* `egui_extras::install_image_loaders`, since
//! egui asks the most recently added loader first; every URI that is not AVIF is
//! declined with [`LoadError::NotSupported`], which hands it to egui_extras.

use std::collections::HashMap;
use std::sync::Arc;

use egui::load::{Bytes, BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use egui::mutex::Mutex;
use egui::{ColorImage, Context};
use zenpixels_convert::PixelBufferConvertTypedExt;

/// The loader's id, namespaced so it cannot collide with egui's own.
pub const LOADER_ID: &str = "limonada::avif::AvifLoader";

/// Install the AVIF loader, once.
pub fn install(ctx: &Context) {
    if !ctx.is_loader_installed(LOADER_ID) {
        ctx.add_image_loader(Arc::new(AvifLoader::default()));
    }
}

/// Whether a URI is one this loader is responsible for.
///
/// Only the extension is available here: by the time an [`ImageLoader`] sees a
/// URI, the content type a server sent has already been consumed by the byte
/// loader. Lemmy serves AVIF from pictrs as `…/image/<uuid>.avif`, so the
/// extension is enough in practice.
pub fn is_avif(uri: &str) -> bool {
    egui::load::has_extension(uri, "avif")
}

/// Decode an AVIF file to the RGBA8 pixels egui draws.
///
/// Returns the reason as a `String` rather than panicking: a decoder meeting a
/// file it cannot handle is a normal outcome for a client that reads pictures
/// from strangers, and the UI turns it into egui's "could not load" glyph.
pub fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    let decoded = zenavif::decode(bytes).map_err(|err| err.to_string())?;
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = rgba.copy_to_contiguous_bytes();

    // `ColorImage::from_rgba_unmultiplied` asserts on a length mismatch, so the
    // mismatch is caught here and reported instead of aborting the app.
    let expected = size[0] * size[1] * 4;
    if pixels.len() != expected {
        return Err(format!(
            "decoded {size:?} but got {} bytes of pixels, expected {expected}",
            pixels.len()
        ));
    }

    Ok(ColorImage::from_rgba_unmultiplied(
        size,
        pixels.as_slice(),
    ))
}

/// The pixel size an AVIF file declares, read from its header without decoding.
///
/// egui uses this to reserve the right space while the decode runs, so a large
/// picture does not start as a 24x24 box and jump.
pub fn size_of(bytes: &[u8]) -> Option<egui::Vec2> {
    let probe = zenavif::detect::probe(bytes).ok()?;
    Some(egui::vec2(probe.width as f32, probe.height as f32))
}

/// What is known about one AVIF URI.
enum Entry {
    /// Decoding on a worker thread. The size comes from the container header.
    Decoding { size: Option<egui::Vec2> },
    Ready(Result<Arc<ColorImage>, String>),
}

#[derive(Default)]
struct AvifLoader {
    cache: Arc<Mutex<HashMap<String, Entry>>>,
}

impl ImageLoader for AvifLoader {
    fn id(&self) -> &str {
        LOADER_ID
    }

    fn load(&self, ctx: &Context, uri: &str, _size_hint: SizeHint) -> ImageLoadResult {
        if !is_avif(uri) {
            return Err(LoadError::NotSupported);
        }

        if let Some(entry) = self.cache.lock().get(uri) {
            return match entry {
                Entry::Decoding { size } => Ok(ImagePoll::Pending { size: *size }),
                Entry::Ready(Ok(image)) => Ok(ImagePoll::Ready {
                    image: Arc::clone(image),
                }),
                Entry::Ready(Err(message)) => Err(LoadError::Loading(message.clone())),
            };
        }

        // First sighting: the bytes come from egui's own byte loaders — the HTTP
        // one from egui_extras, or the context's own included bytes — exactly as
        // egui_extras' loader gets them.
        match ctx.try_load_bytes(uri) {
            Ok(BytesPoll::Ready { bytes, .. }) => {
                let size = size_of(&bytes);
                self.cache
                    .lock()
                    .insert(uri.to_owned(), Entry::Decoding { size });
                spawn_decode(Arc::clone(&self.cache), ctx.clone(), uri.to_owned(), bytes);
                Ok(ImagePoll::Pending { size })
            }
            // Still downloading. Not cached: there is nothing to remember yet,
            // and caching it would keep the entry after the request is dropped.
            Ok(BytesPoll::Pending { .. }) => Ok(ImagePoll::Pending { size: None }),
            // Nothing can supply these bytes, so this URI is not ours to answer
            // for: egui must be free to try the other loaders.
            Err(LoadError::NotSupported | LoadError::NoMatchingBytesLoader) => {
                Err(LoadError::NotSupported)
            }
            Err(err) => Err(err),
        }
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    /// Memory held by decoded pixels, which egui reports as this loader's use.
    /// A still-decoding entry holds nothing yet, since the bytes belong to the
    /// byte loader.
    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .values()
            .map(|entry| match entry {
                Entry::Decoding { .. } | Entry::Ready(Err(_)) => 0,
                Entry::Ready(Ok(image)) => {
                    image.pixels.len() * std::mem::size_of::<egui::Color32>()
                }
            })
            .sum()
    }
}

/// Decode off the UI thread.
///
/// A 440x536 AVIF measured ~150 ms, and a full-size one is far larger, so
/// decoding inline would drop frames. egui's loaders are polled every frame, so
/// the result is simply left in the cache for the next poll.
fn spawn_decode(cache: Arc<Mutex<HashMap<String, Entry>>>, ctx: Context, uri: String, bytes: Bytes) {
    let (thread_cache, thread_uri) = (Arc::clone(&cache), uri.clone());
    let spawned = std::thread::Builder::new()
        .name("limonada-avif".to_owned())
        .spawn(move || {
            let decoded = decode(&bytes).map(Arc::new);
            let mut cache = thread_cache.lock();
            // `forget_image` may have dropped this entry while the decode ran, in
            // which case the result is unwanted and must not resurrect it.
            if let Some(entry) = cache.get_mut(&thread_uri) {
                *entry = Entry::Ready(decoded);
            }
            drop(cache);
            ctx.request_repaint();
        });

    if let Err(err) = spawned {
        // A thread that cannot start must not leave the UI spinning forever.
        if let Some(entry) = cache.lock().get_mut(&uri) {
            *entry = Entry::Ready(Err(format!("could not start a decode thread: {err}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real AVIF captured from a live federated post (`lemmy.zip`, reached via
    /// `lemmy.world`): 4,623 bytes, 440x536, no alpha.
    ///
    /// Deliberately a real file rather than a synthetic one — the MIT-licensed
    /// pure-Rust alternative tried first failed on exactly this picture
    /// ("AV1 tile 0 entropy validation failed"), and nothing generated in a test
    /// would have caught that.
    const REAL_AVIF: &[u8] = include_bytes!("../fixtures/pictures/avif-post.avif");

    #[test]
    fn decodes_a_real_avif_to_rgba_pixels() {
        let image = decode(REAL_AVIF).expect("the captured AVIF should decode");
        assert_eq!(image.size, [440, 536]);

        // A real picture, not a blank buffer: the placeholder a broken decoder
        // would produce is uniform, and this one is not.
        let distinct = image
            .pixels
            .iter()
            .map(|p| p.to_array())
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert!(
            distinct > 100,
            "a real photograph should have many distinct colours, got {distinct}"
        );
    }

    #[test]
    fn the_header_size_matches_what_decoding_produces() {
        let size = size_of(REAL_AVIF).expect("the header should be readable");
        let image = decode(REAL_AVIF).expect("decode");
        assert_eq!(size, egui::vec2(image.size[0] as f32, image.size[1] as f32));
    }

    #[test]
    fn a_file_that_is_not_avif_is_an_error_rather_than_a_panic() {
        assert!(decode(b"not an image at all").is_err());
        // A truncated AVIF is the interesting case: the header parses, the data
        // does not.
        assert!(decode(&REAL_AVIF[..REAL_AVIF.len() / 2]).is_err());
        assert_eq!(size_of(b"no header here"), None);
    }

    #[test]
    fn only_avif_uris_are_claimed() {
        assert!(is_avif("https://lemmy.zip/pictrs/image/abc.avif"));
        assert!(is_avif("https://example.org/a.AVIF"));
        assert!(is_avif("bytes://local.avif#frame=1"));
        assert!(!is_avif("https://lemmy.ml/pictrs/image/abc.png"));
        assert!(!is_avif("https://example.org/photo.avif.jpg"));
        assert!(!is_avif("https://example.org/no-extension"));
    }

    /// The loader as egui uses it: a `bytes://` URI goes through the loader chain
    /// and comes out as a texture of the right size.
    #[test]
    fn the_loader_answers_an_avif_uri_for_egui() {
        let ctx = Context::default();
        install(&ctx);
        ctx.include_bytes("bytes://captured.avif", REAL_AVIF.to_vec());

        let loaded = poll_texture(&ctx, "bytes://captured.avif")
            .expect("the loader should decode a registered AVIF");
        assert_eq!(loaded, egui::vec2(440.0, 536.0));

        // A URI another loader owns must be declined, or egui_extras would never
        // get a chance at it.
        assert!(matches!(
            ctx.try_load_texture(
                "bytes://captured.png",
                egui::TextureOptions::LINEAR,
                SizeHint::default()
            ),
            Err(LoadError::NoMatchingImageLoader { .. }) | Err(LoadError::NoMatchingTextureLoader)
        ));
    }

    /// Drive frames until the loader reports the texture, as the UI does each
    /// frame while a picture is on its way.
    fn poll_texture(ctx: &Context, uri: &str) -> Option<egui::Vec2> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };

        for _ in 0..200 {
            let mut output = ctx.run_ui(input.clone(), |ui| {
                ui.add(egui::Image::new(uri));
            });
            output.textures_delta.clear();

            if let Ok(egui::load::TexturePoll::Ready { texture }) = ctx.try_load_texture(
                uri,
                egui::TextureOptions::LINEAR,
                SizeHint::default(),
            ) {
                return Some(texture.size);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        None
    }
}
