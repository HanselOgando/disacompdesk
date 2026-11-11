//! Compresión y descompresión de vídeo H.264.
//!
//! Una pantalla de 1920×1080 sin comprimir ocupa 8 MB por imagen; a 30
//! imágenes por segundo serían 2 000 Mbit/s. H.264 lo reduce a unos pocos
//! Mbit/s enviando, tras una primera imagen completa, solo las diferencias
//! con la imagen anterior.
//!
//! - [`Encoder`]: imagen BGRA → paquete H.264.
//! - [`Decoder`]: paquete H.264 → imagen BGRA.
//!
//! Ambos trabajan por software con la biblioteca `openh264`, así que
//! funcionan igual en cualquier sistema. No saben nada de red ni de captura.

mod decoder;
mod encoder;

pub use decoder::{Decoder, Image};
pub use encoder::{Encoded, Encoder, EncoderSettings};

/// Bytes por píxel de las imágenes BGRA.
pub const BYTES_PER_PIXEL: usize = 4;

/// Errores del códec.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("la imagen de {width}×{height} debería ocupar {expected} bytes y ocupa {actual}")]
    WrongImageSize {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
    #[error("la imagen de {width}×{height} es demasiado pequeña para comprimirla")]
    ImageTooSmall { width: u32, height: u32 },
    #[error("error del códec H.264: {0}")]
    OpenH264(#[from] openh264::Error),
}
