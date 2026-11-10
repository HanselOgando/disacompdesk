//! Mensajes del protocolo y su formato de trama.
//!
//! Este crate no hace E/S: solo convierte entre [`Message`] y bytes. Así se
//! puede probar sin red y reutilizar sobre cualquier transporte.
//!
//! Formato de una trama:
//!
//! ```text
//! ┌──────────────────────┬─────────────────────────────┐
//! │ longitud (u32, BE)   │ mensaje serializado         │
//! │ 4 bytes              │ `longitud` bytes (postcard) │
//! └──────────────────────┴─────────────────────────────┘
//! ```
//!
//! Un stream QUIC (igual que TCP) entrega un flujo de bytes sin separadores,
//! así que el prefijo de longitud es lo que permite saber dónde acaba cada
//! mensaje.

use serde::{Deserialize, Serialize};

/// Versión del protocolo. Se intercambia en [`Message::Hello`] y debe coincidir
/// en ambos extremos. Hay que incrementarla con cada cambio incompatible.
pub const PROTOCOL_VERSION: u16 = 1;

/// Tamaño máximo del mensaje serializado dentro de una trama.
///
/// Sin este límite, un extremo malicioso podría anunciar una longitud de 4 GB
/// y obligarnos a reservar esa memoria.
pub const MAX_FRAME_LEN: usize = 64 * 1024;

/// Bytes que ocupa el prefijo de longitud.
const HEADER_LEN: usize = 4;

/// Mensajes que intercambian los dos equipos.
///
/// Compatibilidad: postcard identifica cada variante por su posición, así que
/// las variantes nuevas se añaden siempre al final y nunca se reordenan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Primer mensaje en cada sentido: anuncia la versión del protocolo.
    Hello { version: u16 },
    /// Texto libre. Carga útil de prueba de la Etapa 1.
    Text(String),
    /// Cierre voluntario de la sesión (para distinguirlo de una caída de red).
    Bye,
}

/// Errores al codificar o decodificar tramas.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("la trama ocupa {len} bytes y el máximo permitido es {max}")]
    FrameTooLarge { len: usize, max: usize },
    #[error("trama mal formada: {0}")]
    Malformed(#[from] postcard::Error),
    #[error("sobran {0} bytes al final de la trama")]
    TrailingBytes(usize),
}

/// Serializa un mensaje como una trama completa (prefijo + contenido).
pub fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    let payload = postcard::to_stdvec(message)?;
    check_len(payload.len())?;

    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    // `check_len` garantiza que la longitud cabe en un u32.
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Intenta extraer una trama del principio de `buf`.
///
/// - `Ok(Some((mensaje, n)))`: había una trama completa que ocupaba `n` bytes;
///   quien llama debe descartar esos `n` bytes de su búfer.
/// - `Ok(None)`: aún no han llegado todos los bytes; hay que leer más.
/// - `Err(_)`: los datos no son válidos y la conexión debe cerrarse.
pub fn decode(buf: &[u8]) -> Result<Option<(Message, usize)>, ProtocolError> {
    let Some((header, rest)) = buf.split_first_chunk::<HEADER_LEN>() else {
        return Ok(None);
    };
    let len = u32::from_be_bytes(*header) as usize;
    // Se valida la longitud anunciada antes de esperar (y almacenar) el resto.
    check_len(len)?;

    let Some(payload) = rest.get(..len) else {
        return Ok(None);
    };
    let (message, trailing) = postcard::take_from_bytes::<Message>(payload)?;
    if !trailing.is_empty() {
        return Err(ProtocolError::TrailingBytes(trailing.len()));
    }
    Ok(Some((message, HEADER_LEN + len)))
}

fn check_len(len: usize) -> Result<(), ProtocolError> {
    if len > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Message> {
        vec![
            Message::Hello {
                version: PROTOCOL_VERSION,
            },
            Message::Text("hola, ¿qué tal? 👋".to_owned()),
            Message::Text(String::new()),
            Message::Bye,
        ]
    }

    #[test]
    fn roundtrip_of_every_message() {
        for message in samples() {
            let frame = encode(&message).unwrap();
            let (decoded, consumed) = decode(&frame).unwrap().unwrap();
            assert_eq!(decoded, message);
            assert_eq!(consumed, frame.len());
        }
    }

    #[test]
    fn incomplete_frame_asks_for_more_data() {
        let frame = encode(&Message::Text("incompleto".to_owned())).unwrap();
        for cut in 0..frame.len() {
            assert!(
                decode(&frame[..cut]).unwrap().is_none(),
                "con {cut} bytes no debería haber trama completa"
            );
        }
    }

    #[test]
    fn decodes_consecutive_frames_from_one_buffer() {
        let mut buf = Vec::new();
        for message in samples() {
            buf.extend_from_slice(&encode(&message).unwrap());
        }

        let mut decoded = Vec::new();
        while let Some((message, consumed)) = decode(&buf).unwrap() {
            decoded.push(message);
            buf.drain(..consumed);
        }
        assert_eq!(decoded, samples());
        assert!(buf.is_empty());
    }

    #[test]
    fn encode_rejects_oversized_message() {
        let message = Message::Text("x".repeat(MAX_FRAME_LEN + 1));
        assert!(matches!(
            encode(&message),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn decode_rejects_oversized_length_without_waiting_for_payload() {
        // Solo la cabecera, anunciando una trama enorme.
        let header = u32::MAX.to_be_bytes();
        assert!(matches!(
            decode(&header),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn decode_rejects_garbage_payload() {
        // Longitud 1 y una variante de `Message` que no existe.
        let frame = [0, 0, 0, 1, 0xFF];
        assert!(matches!(decode(&frame), Err(ProtocolError::Malformed(_))));
    }

    #[test]
    fn decode_rejects_empty_payload() {
        let frame = [0, 0, 0, 0];
        assert!(matches!(decode(&frame), Err(ProtocolError::Malformed(_))));
    }

    #[test]
    fn decode_rejects_trailing_bytes_inside_frame() {
        let mut payload = postcard::to_stdvec(&Message::Bye).unwrap();
        payload.push(0);
        let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&payload);
        assert!(matches!(
            decode(&frame),
            Err(ProtocolError::TrailingBytes(1))
        ));
    }
}
