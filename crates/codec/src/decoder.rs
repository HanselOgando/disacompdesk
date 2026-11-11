use openh264::formats::YUVSource;

use crate::{BYTES_PER_PIXEL, CodecError};

/// Imagen descomprimida. Toma prestada la memoria del decodificador: hay que
/// usarla antes de descomprimir la siguiente.
#[derive(Debug)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    /// Píxeles BGRA, fila a fila y sin relleno: `width * height * 4` bytes.
    pub data: &'a [u8],
}

/// Descompresor de vídeo. Debe recibir los paquetes en el mismo orden en que
/// se comprimieron, empezando por una imagen completa.
pub struct Decoder {
    inner: openh264::decoder::Decoder,
    pixels: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Result<Self, CodecError> {
        Ok(Self {
            inner: openh264::decoder::Decoder::new()?,
            pixels: Vec::new(),
        })
    }

    /// Descomprime un paquete. Devuelve `None` si el paquete aún no produce
    /// una imagen (por ejemplo, si solo contiene parámetros del vídeo).
    ///
    /// El tamaño de la imagen lo dicta el propio vídeo y puede cambiar de un
    /// paquete a otro.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Image<'_>>, CodecError> {
        let Some(yuv) = self.inner.decode(packet)? else {
            return Ok(None);
        };
        let (width, height) = yuv.dimensions();

        self.pixels.resize(width * height * BYTES_PER_PIXEL, 0);
        yuv.write_rgba8(&mut self.pixels);
        // La biblioteca entrega rojo-verde-azul; el resto del proyecto usa
        // azul-verde-rojo, como la captura de Windows. Se intercambian el
        // primer y el tercer byte de cada píxel.
        for pixel in self.pixels.as_chunks_mut::<BYTES_PER_PIXEL>().0 {
            pixel.swap(0, 2);
        }

        Ok(Some(Image {
            width: width as u32,
            height: height as u32,
            data: &self.pixels,
        }))
    }
}
