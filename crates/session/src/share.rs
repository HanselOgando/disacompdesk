use std::time::{Duration, Instant};

use disacomp_capture::{CaptureError, FramePacer, ScreenCapturer};
use disacomp_codec::{Encoder, EncoderSettings};
use disacomp_transport::MediaSender;
use tokio::sync::mpsc;

use crate::{SessionError, now_micros};

/// Espera máxima a que la pantalla cambie. Es también lo que tarda el hilo de
/// captura, como mucho, en darse cuenta de que la sesión terminó.
const CAPTURE_WAIT: Duration = Duration::from_millis(100);

/// Ajustes de una pantalla compartida.
#[derive(Debug, Clone, Copy)]
pub struct ShareSettings {
    /// Máximo de imágenes por segundo.
    pub fps: u32,
    /// Caudal objetivo del vídeo en kilobits por segundo.
    pub bitrate_kbps: u32,
}

impl Default for ShareSettings {
    fn default() -> Self {
        Self {
            fps: 30,
            bitrate_kbps: 6000,
        }
    }
}

/// Imagen ya comprimida, de camino del hilo de captura a la red.
struct Packet {
    keyframe: bool,
    capture_time_micros: u64,
    data: Vec<u8>,
}

/// Captura, comprime y envía la pantalla hasta que algo falle o se cancele.
///
/// `open_capturer` se ejecuta dentro del hilo de captura: un capturador debe
/// usarse en el hilo en que se creó, así que no se puede crear fuera y
/// pasarlo.
///
/// Para dejar de compartir basta con cancelar esta función (por ejemplo, al
/// perder en un `tokio::select!`): el hilo de captura lo nota y termina solo.
pub async fn share<F>(
    open_capturer: F,
    mut media: MediaSender,
    settings: ShareSettings,
) -> Result<(), SessionError>
where
    F: FnOnce() -> Result<Box<dyn ScreenCapturer>, CaptureError> + Send + 'static,
{
    let (packets, mut outgoing) = mpsc::channel::<Packet>(1);
    let worker =
        tokio::task::spawn_blocking(move || capture_loop(open_capturer, settings, &packets));

    // El canal se cierra cuando el hilo de captura termina.
    while let Some(packet) = outgoing.recv().await {
        media
            .send(packet.keyframe, packet.capture_time_micros, &packet.data)
            .await?;
    }
    worker.await.map_err(|_| SessionError::WorkerPanicked)?
}

fn capture_loop<F>(
    open_capturer: F,
    settings: ShareSettings,
    packets: &mpsc::Sender<Packet>,
) -> Result<(), SessionError>
where
    F: FnOnce() -> Result<Box<dyn ScreenCapturer>, CaptureError>,
{
    let mut capturer = open_capturer()?;
    let mut encoder = Encoder::new(EncoderSettings {
        bitrate_kbps: settings.bitrate_kbps,
        max_fps: settings.fps,
    })?;
    let mut pacer = FramePacer::new(settings.fps, Instant::now());

    // `is_closed` pasa a ser cierto cuando `share` termina o se cancela.
    while !packets.is_closed() {
        std::thread::sleep(pacer.schedule(Instant::now()));
        let Some(frame) = capturer.next_frame(CAPTURE_WAIT)? else {
            continue;
        };
        let capture_time_micros = now_micros();
        let Some(encoded) = encoder.encode(frame.width, frame.height, frame.data)? else {
            continue;
        };
        let packet = Packet {
            keyframe: encoded.keyframe,
            capture_time_micros,
            data: encoded.data.to_vec(),
        };
        // Aquí está la contrapresión: si la red aún no se ha llevado el
        // paquete anterior, este hilo espera y, mientras, no captura.
        if packets.blocking_send(packet).is_err() {
            break;
        }
    }
    Ok(())
}
