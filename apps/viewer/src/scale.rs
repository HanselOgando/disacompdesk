//! Ajuste de la imagen recibida al tamaño de la ventana.

/// Pinta la imagen BGRA `src` (`src_w` × `src_h`) dentro de `dst`
/// (`dst_w` × `dst_h`, un `u32` por píxel con formato `0x00RRGGBB`).
///
/// La imagen se ajusta entera manteniendo su proporción y queda centrada; lo
/// que sobra de ventana se deja en negro. Para cada píxel de destino se toma
/// el píxel de origen más cercano: es el método más simple y rápido, aunque
/// no el más fino.
pub fn blit(src: &[u8], src_w: usize, src_h: usize, dst: &mut [u32], dst_w: usize, dst_h: usize) {
    dst.fill(0);
    if src_w == 0
        || src_h == 0
        || dst_w == 0
        || dst_h == 0
        || src.len() < src_w * src_h * 4
        || dst.len() < dst_w * dst_h
    {
        return;
    }

    // Tamaño que ocupará la imagen: llena la ventana a lo ancho o a lo alto,
    // lo que antes tope.
    let (width, height) = if dst_w * src_h <= dst_h * src_w {
        (dst_w, (src_h * dst_w / src_w).max(1))
    } else {
        ((src_w * dst_h / src_h).max(1), dst_h)
    };
    let (left, top) = ((dst_w - width) / 2, (dst_h - height) / 2);

    // Qué columna de origen corresponde a cada columna de destino. Se calcula
    // una vez y sirve para todas las filas.
    let columns: Vec<usize> = (0..width).map(|x| x * src_w / width * 4).collect();

    for y in 0..height {
        let source_row = &src[(y * src_h / height) * src_w * 4..][..src_w * 4];
        let target_row = &mut dst[(top + y) * dst_w + left..][..width];
        for (pixel, &column) in target_row.iter_mut().zip(&columns) {
            *pixel = u32::from(source_row[column])
                | u32::from(source_row[column + 1]) << 8
                | u32::from(source_row[column + 2]) << 16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::blit;

    const RED: u32 = 0x00FF_0000;
    const BLUE: u32 = 0x0000_00FF;

    /// Imagen de 2×1: un píxel rojo y uno azul (en bytes BGRA).
    const RED_BLUE: [u8; 8] = [0, 0, 255, 255, 255, 0, 0, 255];

    #[test]
    fn same_size_copies_pixels_converting_the_format() {
        let mut dst = [1u32; 2];
        blit(&RED_BLUE, 2, 1, &mut dst, 2, 1);
        assert_eq!(dst, [RED, BLUE]);
    }

    #[test]
    fn enlarges_keeping_proportion_and_centres_with_black_bars() {
        // Ventana de 4×4 para una imagen 2:1 → ocupa 4×2, centrada en vertical.
        let mut dst = [1u32; 16];
        blit(&RED_BLUE, 2, 1, &mut dst, 4, 4);
        #[rustfmt::skip]
        assert_eq!(dst, [
            0,   0,   0,    0,
            RED, RED, BLUE, BLUE,
            RED, RED, BLUE, BLUE,
            0,   0,   0,    0,
        ]);
    }

    #[test]
    fn shrinks_and_centres_horizontally() {
        // Ventana de 4×1 para una imagen 2:1 → ocupa 2×1, centrada.
        let mut dst = [1u32; 4];
        blit(&RED_BLUE, 2, 1, &mut dst, 4, 1);
        assert_eq!(dst, [0, RED, BLUE, 0]);
    }

    #[test]
    fn inconsistent_sizes_leave_the_window_black() {
        let mut dst = [1u32; 4];
        blit(&RED_BLUE, 100, 100, &mut dst, 2, 2);
        assert_eq!(dst, [0; 4]);
        blit(&[], 0, 0, &mut dst, 2, 2);
        assert_eq!(dst, [0; 4]);
    }
}
