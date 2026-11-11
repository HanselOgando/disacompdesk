//! Formato de los paquetes de vídeo.
//!
//! El vídeo viaja por su propio stream, separado del de mensajes, con un
//! formato fijo en lugar de postcard: así los datos comprimidos (que pueden
//! ocupar cientos de kilobytes) se escriben tal cual, sin copias intermedias.
//!
//! ```text
//! ┌────────────────┬───────────┬──────────────────────┬───────────────┐
//! │ longitud (u32) │ marcas    │ hora de captura (u64)│ datos H.264   │
//! │ 4 bytes, BE    │ 1 byte    │ 8 bytes, BE          │ `longitud`    │
//! └────────────────┴───────────┴──────────────────────┴───────────────┘
//! ```

use crate::ProtocolError;

/// Tamaño máximo de los datos de un paquete. Un fotograma completo de una
/// pantalla 4K muy detallada ronda 1 MB; esto deja margen de sobra sin
/// permitir que el otro extremo nos haga reservar memoria sin límite.
pub const MAX_PACKET_LEN: usize = 4 * 1024 * 1024;

/// Bytes que ocupa la cabecera.
pub const HEADER_LEN: usize = 13;

/// Marca: el paquete es un fotograma completo (no depende de los anteriores).
const FLAG_KEYFRAME: u8 = 0b0000_0001;

/// Un fotograma de vídeo comprimido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPacket {
    /// Fotograma completo: se puede descomprimir sin haber visto los anteriores.
    pub keyframe: bool,
    /// Momento de la captura, en microsegundos desde 1970 según el reloj de
    /// quien comparte. Sirve para medir la latencia.
    pub capture_time_micros: u64,
    /// Datos H.264.
    pub data: Vec<u8>,
}

/// Construye la cabecera de un paquete cuyos datos ocupan `data_len` bytes.
///
/// Se devuelve aparte de los datos para que quien envía pueda escribir
/// primero la cabecera y luego los datos directamente, sin juntarlos antes
/// en otro búfer.
pub fn encode_header(
    keyframe: bool,
    capture_time_micros: u64,
    data_len: usize,
) -> Result<[u8; HEADER_LEN], ProtocolError> {
    check_len(data_len)?;
    let mut header = [0u8; HEADER_LEN];
    // `check_len` garantiza que la longitud cabe en un u32.
    header[0..4].copy_from_slice(&(data_len as u32).to_be_bytes());
    header[4] = if keyframe { FLAG_KEYFRAME } else { 0 };
    header[5..13].copy_from_slice(&capture_time_micros.to_be_bytes());
    Ok(header)
}

/// Intenta extraer un paquete del principio de `buf`. Mismo contrato que
/// [`crate::decode`]: `Ok(None)` significa que faltan bytes por llegar.
pub fn decode(buf: &[u8]) -> Result<Option<(VideoPacket, usize)>, ProtocolError> {
    let Some((header, rest)) = buf.split_first_chunk::<HEADER_LEN>() else {
        return Ok(None);
    };
    let (len, header) = header
        .split_first_chunk::<4>()
        .expect("la cabecera mide más de 4 bytes");
    let len = u32::from_be_bytes(*len) as usize;
    // Se valida la longitud anunciada antes de esperar (y almacenar) los datos.
    check_len(len)?;

    let Some(data) = rest.get(..len) else {
        return Ok(None);
    };
    // Las marcas desconocidas se ignoran, para poder añadir más en el futuro.
    let keyframe = header[0] & FLAG_KEYFRAME != 0;
    let (time, _) = header[1..]
        .split_first_chunk::<8>()
        .expect("quedan exactamente 8 bytes");

    let packet = VideoPacket {
        keyframe,
        capture_time_micros: u64::from_be_bytes(*time),
        data: data.to_vec(),
    };
    Ok(Some((packet, HEADER_LEN + len)))
}

fn check_len(len: usize) -> Result<(), ProtocolError> {
    if len > MAX_PACKET_LEN {
        return Err(ProtocolError::FrameTooLarge {
            len,
            max: MAX_PACKET_LEN,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(packet: &VideoPacket) -> Vec<u8> {
        let header = encode_header(
            packet.keyframe,
            packet.capture_time_micros,
            packet.data.len(),
        )
        .unwrap();
        [header.as_slice(), &packet.data].concat()
    }

    fn sample(keyframe: bool, len: usize) -> VideoPacket {
        VideoPacket {
            keyframe,
            capture_time_micros: 1_762_900_000_123_456,
            data: (0..len).map(|i| i as u8).collect(),
        }
    }

    #[test]
    fn roundtrip() {
        for packet in [sample(true, 1000), sample(false, 1), sample(false, 0)] {
            let bytes = frame(&packet);
            let (decoded, consumed) = decode(&bytes).unwrap().unwrap();
            assert_eq!(decoded, packet);
            assert_eq!(consumed, bytes.len());
        }
    }

    #[test]
    fn incomplete_packet_asks_for_more_data() {
        let bytes = frame(&sample(true, 50));
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).unwrap().is_none());
        }
    }

    #[test]
    fn decodes_consecutive_packets() {
        let packets = [sample(true, 300), sample(false, 20), sample(false, 5)];
        let mut buf: Vec<u8> = packets.iter().flat_map(frame).collect();

        let mut decoded = Vec::new();
        while let Some((packet, consumed)) = decode(&buf).unwrap() {
            decoded.push(packet);
            buf.drain(..consumed);
        }
        assert_eq!(decoded, packets);
    }

    #[test]
    fn rejects_oversized_packets() {
        assert!(matches!(
            encode_header(false, 0, MAX_PACKET_LEN + 1),
            Err(ProtocolError::FrameTooLarge { .. })
        ));

        let mut header = encode_header(false, 0, 0).unwrap();
        header[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            decode(&header),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn unknown_flags_are_ignored() {
        let mut bytes = frame(&sample(true, 4));
        bytes[4] |= 0b1000_0000;
        let (decoded, _) = decode(&bytes).unwrap().unwrap();
        assert!(decoded.keyframe);
    }
}
