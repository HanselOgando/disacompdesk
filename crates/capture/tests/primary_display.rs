//! Pruebas contra la pantalla real. Necesitan una sesión de escritorio de
//! Windows abierta (no funcionan en un servidor sin pantalla).
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use disacomp_capture::{BYTES_PER_PIXEL, Rect, list_displays, open};

#[test]
fn lists_at_least_one_display_with_a_primary() {
    let displays = list_displays().unwrap();
    assert!(!displays.is_empty());
    assert!(displays.iter().any(|display| display.primary));
    for (position, display) in displays.iter().enumerate() {
        assert_eq!(display.index, position);
        assert!(display.width > 0 && display.height > 0);
        assert!(!display.name.is_empty());
    }
}

#[test]
fn first_frame_is_a_full_non_empty_image() {
    let mut capturer = open(0).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);

    // El primer fotograma llega enseguida, pero se permite algún "sin cambios".
    let (width, height, len, dirty, has_content) = loop {
        assert!(
            Instant::now() < deadline,
            "no llegó ningún fotograma en 2 s"
        );
        if let Some(frame) = capturer.next_frame(Duration::from_millis(200)).unwrap() {
            break (
                frame.width,
                frame.height,
                frame.data.len(),
                frame.dirty.to_vec(),
                frame.data.iter().any(|&byte| byte != 0),
            );
        }
    };

    assert_eq!(
        (width, height),
        (capturer.display().width, capturer.display().height)
    );
    assert_eq!(len, width as usize * height as usize * BYTES_PER_PIXEL);
    assert_eq!(dirty, vec![Rect::full(width, height)]);
    assert!(has_content, "la imagen está completamente negra");
}

#[test]
fn later_frames_keep_the_same_size_and_report_valid_rects() {
    let mut capturer = open(0).unwrap();
    let mut size = None;
    let deadline = Instant::now() + Duration::from_secs(1);

    while Instant::now() < deadline {
        let Some(frame) = capturer.next_frame(Duration::from_millis(100)).unwrap() else {
            continue;
        };
        let current = (frame.width, frame.height);
        assert_eq!(*size.get_or_insert(current), current);
        assert!(!frame.dirty.is_empty());
        for rect in frame.dirty {
            assert!(rect.width > 0 && rect.height > 0);
            assert!(rect.x + rect.width <= frame.width);
            assert!(rect.y + rect.height <= frame.height);
        }
    }
    assert!(size.is_some(), "no llegó ningún fotograma");
}

#[test]
fn unknown_display_is_reported() {
    assert!(matches!(
        open(9999),
        Err(disacomp_capture::CaptureError::DisplayNotFound(9999))
    ));
}
