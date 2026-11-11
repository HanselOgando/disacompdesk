//! Escritura de imágenes BMP sin comprimir.
//!
//! Es el formato de imagen más simple que existe: una cabecera de 54 bytes
//! seguida de los píxeles. Basta para inspeccionar capturas de prueba sin
//! añadir una biblioteca de imágenes al proyecto.

use anyhow::{Result, ensure};

const FILE_HEADER_LEN: usize = 14;
const INFO_HEADER_LEN: usize = 40;
const HEADERS_LEN: usize = FILE_HEADER_LEN + INFO_HEADER_LEN;
/// 72 puntos por pulgada, expresado en píxeles por metro.
const PIXELS_PER_METER: u32 = 2835;

/// Codifica una imagen BGRA (4 bytes por píxel, de arriba abajo) como BMP de
/// 24 bits.
pub fn encode(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    ensure!(
        bgra.len() == w * h * 4,
        "la imagen debería ocupar {} bytes y ocupa {}",
        w * h * 4,
        bgra.len()
    );
    ensure!(
        i32::try_from(width).is_ok() && i32::try_from(height).is_ok(),
        "la imagen es demasiado grande para el formato BMP"
    );

    // Cada fila se rellena con ceros hasta un múltiplo de 4 bytes.
    let row_len = (w * 3).next_multiple_of(4);
    let pixels_len = row_len * h;
    let file_len = HEADERS_LEN + pixels_len;
    ensure!(
        u32::try_from(file_len).is_ok(),
        "la imagen es demasiado grande para el formato BMP"
    );

    let mut out = Vec::with_capacity(file_len);
    // Cabecera de archivo.
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(HEADERS_LEN as u32).to_le_bytes());
    // Cabecera de imagen.
    out.extend_from_slice(&(INFO_HEADER_LEN as u32).to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planos
    out.extend_from_slice(&24u16.to_le_bytes()); // bits por píxel
    out.extend_from_slice(&0u32.to_le_bytes()); // sin compresión
    out.extend_from_slice(&(pixels_len as u32).to_le_bytes());
    out.extend_from_slice(&PIXELS_PER_METER.to_le_bytes());
    out.extend_from_slice(&PIXELS_PER_METER.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // colores de paleta
    out.extend_from_slice(&0u32.to_le_bytes()); // colores importantes

    // Píxeles: BMP guarda las filas de abajo arriba y sin el cuarto byte.
    let padding = [0u8; 3];
    for row in bgra.chunks_exact(w * 4).rev() {
        for [blue, green, red, _unused] in row.as_chunks::<4>().0 {
            out.extend_from_slice(&[*blue, *green, *red]);
        }
        out.extend_from_slice(&padding[..row_len - w * 3]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_header_and_flips_rows() {
        // Imagen de 3×2: la fila superior azul y la inferior roja (en BGRA).
        let blue = [255, 0, 0, 0];
        let red = [0, 0, 255, 0];
        let bgra = [blue, blue, blue, red, red, red].concat();

        let bmp = encode(3, 2, &bgra).unwrap();

        // 3 píxeles × 3 bytes = 9, redondeado a 12 por fila; dos filas.
        assert_eq!(bmp.len(), 54 + 24);
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()), 78);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().unwrap()), 3);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bmp[28..30].try_into().unwrap()), 24);

        // La primera fila guardada es la inferior (roja), con su relleno.
        assert_eq!(&bmp[54..66], &[0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0]);
        // Después, la superior (azul).
        assert_eq!(&bmp[66..78], &[255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_data_of_the_wrong_size() {
        assert!(encode(2, 2, &[0; 15]).is_err());
    }
}
