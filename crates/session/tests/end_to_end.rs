//! Sesión completa por 127.0.0.1 con una pantalla inventada: captura falsa →
//! compresión → red → descompresión → imágenes recibidas.
#![allow(clippy::unwrap_used)]

use std::net::Ipv4Addr;
use std::time::Duration;

use disacomp_capture::{CaptureError, DisplayInfo, Frame, Rect, ScreenCapturer};
use disacomp_session::{SessionError, ShareSettings, share, watch};
use disacomp_transport::{Connection, Identity, Listener, TransportOptions, connect};
use tokio::sync::mpsc;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(20);
const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

/// Pantalla de mentira: entrega `remaining` imágenes distintas y después se
/// comporta como una pantalla que ya no cambia.
struct FakeScreen {
    info: DisplayInfo,
    pixels: Vec<u8>,
    dirty: [Rect; 1],
    step: usize,
    remaining: usize,
}

impl FakeScreen {
    fn new(frames: usize) -> Self {
        Self {
            info: DisplayInfo {
                index: 0,
                name: "falsa".to_owned(),
                width: WIDTH,
                height: HEIGHT,
                primary: true,
            },
            pixels: vec![0; (WIDTH * HEIGHT * 4) as usize],
            dirty: [Rect::full(WIDTH, HEIGHT)],
            step: 0,
            remaining: frames,
        }
    }
}

impl ScreenCapturer for FakeScreen {
    fn display(&self) -> &DisplayInfo {
        &self.info
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame<'_>>, CaptureError> {
        if self.remaining == 0 {
            std::thread::sleep(timeout);
            return Ok(None);
        }
        self.remaining -= 1;
        self.step += 1;
        // Fondo gris uniforme que se aclara un poco en cada imagen.
        self.pixels.fill(40 + self.step as u8 * 10);
        Ok(Some(Frame {
            width: WIDTH,
            height: HEIGHT,
            data: &self.pixels,
            dirty: &self.dirty,
            copy_time: Duration::ZERO,
        }))
    }
}

async fn connected_pair() -> (Listener, Connection, Connection) {
    let identity = Identity::generate().unwrap();
    let options = TransportOptions::default();
    let listener =
        Listener::bind((Ipv4Addr::LOCALHOST, 0).into(), &identity, options.clone()).unwrap();
    let addr = listener.local_addr().unwrap();
    let (server, client) = tokio::join!(
        listener.accept(),
        connect(addr, identity.fingerprint(), &options)
    );
    (listener, server.unwrap(), client.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_screen_reaches_the_watcher() {
    const FRAMES: usize = 8;
    let (_listener, server, client) = connected_pair().await;

    let sender = server.media().open().await.unwrap();
    let sharing = tokio::spawn(share(
        || Ok(Box::new(FakeScreen::new(FRAMES)) as Box<dyn ScreenCapturer>),
        sender,
        ShareSettings {
            fps: 60,
            bitrate_kbps: 2000,
        },
    ));

    // Cada imagen recibida se resume como (ancho, alto, gris medio, latencia).
    let (seen, mut pictures) = mpsc::unbounded_channel();
    let receiver = timeout(TEST_TIMEOUT, client.media().accept())
        .await
        .unwrap()
        .unwrap();
    let watching = tokio::spawn(watch(receiver, move |picture| {
        let sum: u64 = picture.data.iter().step_by(4).map(|&b| u64::from(b)).sum();
        let mean = sum / u64::from(picture.width * picture.height);
        let _ = seen.send((picture.width, picture.height, mean, picture.latency));
    }));

    for step in 1..=FRAMES as u64 {
        let (width, height, mean, latency) = timeout(TEST_TIMEOUT, pictures.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((width, height), (WIDTH, HEIGHT));
        // Llegan en orden y con el gris que tocaba, salvo pérdida de compresión.
        assert!(
            mean.abs_diff(40 + step * 10) <= 4,
            "imagen {step}: gris {mean}"
        );
        assert!(latency < Duration::from_secs(5));
    }

    // Al dejar de compartir, quien mira termina limpiamente.
    sharing.abort();
    let _ = sharing.await;
    let result = timeout(TEST_TIMEOUT, watching).await.unwrap().unwrap();
    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_failure_ends_the_share_with_its_error() {
    let (_listener, server, _client) = connected_pair().await;
    let sender = server.media().open().await.unwrap();

    let result = timeout(
        TEST_TIMEOUT,
        share(
            || Err(CaptureError::DisplayNotFound(7)),
            sender,
            ShareSettings::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(SessionError::Capture(CaptureError::DisplayNotFound(7)))
    ));
}
