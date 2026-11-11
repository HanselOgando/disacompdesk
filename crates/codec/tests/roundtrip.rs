//! Comprime imágenes sintéticas, las descomprime y comprueba que el
//! resultado se parece al original.
#![allow(clippy::unwrap_used)]

use disacomp_codec::{CodecError, Decoder, Encoder, EncoderSettings};

/// Imagen de prueba: un degradado suave con un cuadrado blanco que se
/// desplaza según `step`, para que cada imagen difiera un poco de la anterior.
fn test_image(width: usize, height: usize, step: usize) -> Vec<u8> {
    let mut bgra = vec![0u8; width * height * 4];
    let square = (step * 8) % (width - 40);
    for y in 0..height {
        for x in 0..width {
            let pixel = &mut bgra[(y * width + x) * 4..][..4];
            let inside = (square..square + 32).contains(&x) && (20..52).contains(&y);
            let (b, g, r) = if inside {
                (255, 255, 255)
            } else {
                ((x * 255 / width) as u8, (y * 255 / height) as u8, 64)
            };
            pixel.copy_from_slice(&[b, g, r, 255]);
        }
    }
    bgra
}

/// Diferencia media por canal de color (0 = idénticas, 255 = opuestas),
/// ignorando el cuarto byte, que no lleva información.
fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let total: u64 = a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .flat_map(|(pa, pb)| (0..3).map(move |c| u64::from(pa[c].abs_diff(pb[c]))))
        .sum();
    total as f64 / (a.len() / 4 * 3) as f64
}

fn settings() -> EncoderSettings {
    EncoderSettings {
        bitrate_kbps: 2000,
        max_fps: 30,
    }
}

#[test]
fn decoded_video_resembles_the_original() {
    let (width, height) = (320, 240);
    let mut encoder = Encoder::new(settings()).unwrap();
    let mut decoder = Decoder::new().unwrap();
    let mut decoded_frames = 0;

    for step in 0..10 {
        let original = test_image(width, height, step);
        let encoded = encoder
            .encode(width as u32, height as u32, &original)
            .unwrap()
            .expect("cada imagen debe producir datos");

        assert_eq!(encoded.keyframe, step == 0, "solo la primera es completa");
        // Comprimir debe compensar: muy por debajo de los 307 200 bytes originales.
        assert!(encoded.data.len() < original.len() / 4);

        let image = decoder
            .decode(encoded.data)
            .unwrap()
            .expect("cada paquete debe producir una imagen");
        assert_eq!((image.width, image.height), (width as u32, height as u32));

        let difference = mean_difference(&original, image.data);
        assert!(
            difference < 6.0,
            "imagen {step}: diferencia media {difference:.1}"
        );
        decoded_frames += 1;
    }
    assert_eq!(decoded_frames, 10);
}

#[test]
fn odd_dimensions_are_cropped_to_even() {
    let (width, height) = (321, 241);
    let mut encoder = Encoder::new(settings()).unwrap();
    let mut decoder = Decoder::new().unwrap();

    let original = test_image(width, height, 0);
    let encoded = encoder
        .encode(width as u32, height as u32, &original)
        .unwrap()
        .unwrap();
    let image = decoder.decode(encoded.data).unwrap().unwrap();
    assert_eq!((image.width, image.height), (320, 240));

    // Debe coincidir con el original sin su última columna y fila.
    let cropped: Vec<u8> = original
        .chunks_exact(width * 4)
        .take(240)
        .flat_map(|row| &row[..320 * 4])
        .copied()
        .collect();
    assert!(mean_difference(&cropped, image.data) < 6.0);
}

#[test]
fn resolution_change_restarts_with_a_keyframe() {
    let mut encoder = Encoder::new(settings()).unwrap();
    let mut decoder = Decoder::new().unwrap();

    for (index, (width, height)) in [(320, 240), (320, 240), (640, 480), (640, 480)]
        .into_iter()
        .enumerate()
    {
        let original = test_image(width, height, index);
        let encoded = encoder
            .encode(width as u32, height as u32, &original)
            .unwrap()
            .unwrap();
        // Las imágenes 0 y 2 estrenan resolución.
        assert_eq!(encoded.keyframe, index % 2 == 0, "imagen {index}");

        let image = decoder.decode(encoded.data).unwrap().unwrap();
        assert_eq!((image.width, image.height), (width as u32, height as u32));
        assert!(mean_difference(&original, image.data) < 6.0);
    }
}

#[test]
fn rejects_images_with_the_wrong_amount_of_data() {
    let mut encoder = Encoder::new(settings()).unwrap();
    assert!(matches!(
        encoder.encode(320, 240, &[0; 100]),
        Err(CodecError::WrongImageSize { .. })
    ));
    assert!(matches!(
        encoder.encode(1, 1, &[0; 4]),
        Err(CodecError::ImageTooSmall { .. })
    ));
}
