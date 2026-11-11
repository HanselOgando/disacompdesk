use std::time::Instant;

use openh264::encoder::{
    BitRate, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode, UsageType,
};
use openh264::formats::{BgraSliceU8, YUVBuffer};
use openh264::{OpenH264API, Timestamp};

use crate::{BYTES_PER_PIXEL, CodecError};

/// Ajustes de compresión.
#[derive(Debug, Clone, Copy)]
pub struct EncoderSettings {
    /// Caudal objetivo en kilobits por segundo. Más caudal, más nitidez.
    pub bitrate_kbps: u32,
    /// Ritmo máximo de imágenes que se le van a entregar.
    pub max_fps: u32,
}

impl Default for EncoderSettings {
    fn default() -> Self {
        Self {
            bitrate_kbps: 6000,
            max_fps: 30,
        }
    }
}

/// Resultado de comprimir una imagen. Toma prestada la memoria del
/// codificador: hay que usarlo antes de comprimir la siguiente.
#[derive(Debug)]
pub struct Encoded<'a> {
    /// Imagen completa: se puede descomprimir sin las anteriores.
    pub keyframe: bool,
    /// Datos H.264.
    pub data: &'a [u8],
}

/// Compresor de vídeo. Cada imagen se comprime en relación con la anterior,
/// así que hay que entregárselas en orden y sin saltarse ninguna de las que
/// devuelve.
pub struct Encoder {
    inner: openh264::encoder::Encoder,
    /// H.264 no trabaja con rojo/verde/azul sino con brillo y color por
    /// separado (YUV), con el color a media resolución. Este búfer guarda la
    /// imagen ya convertida.
    yuv: YUVBuffer,
    /// Dimensiones (pares) de `yuv`.
    size: (usize, usize),
    /// Copia recortada de la imagen, solo para anchuras impares.
    cropped: Vec<u8>,
    output: Vec<u8>,
    started: Instant,
}

impl Encoder {
    pub fn new(settings: EncoderSettings) -> Result<Self, CodecError> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(
                settings.bitrate_kbps.saturating_mul(1000),
            ))
            .max_frame_rate(FrameRate::from_hz(settings.max_fps.max(1) as f32))
            // Ajustes internos pensados para pantallas (texto, bordes nítidos,
            // grandes zonas iguales) en lugar de imagen de cámara.
            .usage_type(UsageType::ScreenContentRealTime)
            // Dos mejoras de calidad que la biblioteca no admite con
            // contenido de pantalla; se apagan para que no avise en cada uso.
            .adaptive_quantization(false)
            .background_detection(false)
            // Nunca descartar imágenes para ahorrar caudal: si la pantalla
            // deja de cambiar justo después, el cambio descartado no se
            // enviaría nunca. Es preferible que baje la nitidez un instante,
            // que es lo que hace este modo de control de caudal.
            .skip_frames(false)
            .rate_control_mode(RateControlMode::Bufferbased)
            // Sin imágenes completas periódicas: el transporte es fiable y no
            // pierde datos, así que basta con la primera.
            .intra_frame_period(IntraFramePeriod::auto());

        let inner =
            openh264::encoder::Encoder::with_api_config(OpenH264API::from_source(), config)?;
        Ok(Self {
            inner,
            yuv: YUVBuffer::new(2, 2),
            size: (2, 2),
            cropped: Vec::new(),
            output: Vec::new(),
            started: Instant::now(),
        })
    }

    /// Comprime una imagen BGRA de `width` × `height` (4 bytes por píxel, fila
    /// a fila y sin relleno).
    ///
    /// Si el tamaño cambia respecto a la imagen anterior, el vídeo se
    /// reinicia con una imagen completa. Devuelve `None` si el compresor no
    /// produjo datos para esta imagen.
    pub fn encode(
        &mut self,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Result<Option<Encoded<'_>>, CodecError> {
        let (w, h) = (width as usize, height as usize);
        let expected = w * h * BYTES_PER_PIXEL;
        if bgra.len() != expected {
            return Err(CodecError::WrongImageSize {
                width,
                height,
                expected,
                actual: bgra.len(),
            });
        }
        // H.264 guarda el color a media resolución, así que necesita anchura
        // y altura pares: si no lo son, se descarta la última columna o fila.
        let (even_w, even_h) = (w & !1, h & !1);
        if even_w < 2 || even_h < 2 {
            return Err(CodecError::ImageTooSmall { width, height });
        }

        if self.size != (even_w, even_h) {
            self.yuv = YUVBuffer::new(even_w, even_h);
            self.size = (even_w, even_h);
        }

        let pixels: &[u8] = if even_w == w {
            // Quitar la última fila es solo acortar el bloque de memoria.
            &bgra[..even_w * even_h * BYTES_PER_PIXEL]
        } else {
            // Quitar la última columna obliga a copiar fila a fila.
            self.cropped.clear();
            for row in bgra.chunks_exact(w * BYTES_PER_PIXEL).take(even_h) {
                self.cropped
                    .extend_from_slice(&row[..even_w * BYTES_PER_PIXEL]);
            }
            &self.cropped
        };
        self.yuv
            .read_bgra8(BgraSliceU8::new(pixels, (even_w, even_h)));

        // La hora real de cada imagen permite al control de caudal acertar
        // aunque lleguen a un ritmo irregular.
        let timestamp = Timestamp::from_millis(self.started.elapsed().as_millis() as u64);
        let bitstream = self.inner.encode_at(&self.yuv, timestamp)?;
        let keyframe = match bitstream.frame_type() {
            FrameType::IDR | FrameType::I => true,
            FrameType::P | FrameType::IPMixed => false,
            FrameType::Skip | FrameType::Invalid => return Ok(None),
        };

        self.output.clear();
        bitstream.write_vec(&mut self.output);
        if self.output.is_empty() {
            return Ok(None);
        }
        Ok(Some(Encoded {
            keyframe,
            data: &self.output,
        }))
    }
}
