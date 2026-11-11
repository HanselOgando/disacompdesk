use std::time::Duration;

use disacomp_codec::Decoder;
use disacomp_protocol::media::VideoPacket;
use disacomp_transport::{MediaReceiver, TransportError};
use tokio::sync::mpsc;

use crate::{SessionError, now_micros};

/// Imagen recibida, lista para mostrar. Toma prestada la memoria del
/// descompresor: quien quiera conservarla debe copiarla.
#[derive(Debug)]
pub struct Picture<'a> {
    pub width: u32,
    pub height: u32,
    /// Píxeles BGRA, fila a fila y sin relleno.
    pub data: &'a [u8],
    /// Tiempo desde que se capturó hasta que quedó lista para mostrar. Solo
    /// es exacto si los dos equipos tienen el reloj igual (o son el mismo).
    pub latency: Duration,
    /// Tamaño comprimido con el que viajó por la red.
    pub compressed_len: usize,
}

/// Recibe el vídeo y llama a `on_picture` con cada imagen, hasta que el otro
/// equipo deja de compartir (devuelve `Ok`), algo falla o se cancela.
///
/// `on_picture` se ejecuta en el hilo de descompresión, no en el runtime
/// asíncrono, así que puede hacer trabajo bloqueante; mientras no vuelva, no
/// se descomprime la imagen siguiente.
pub async fn watch<F>(mut media: MediaReceiver, on_picture: F) -> Result<(), SessionError>
where
    F: FnMut(Picture<'_>) + Send + 'static,
{
    let (packets, incoming) = mpsc::channel::<VideoPacket>(1);
    let worker = tokio::task::spawn_blocking(move || decode_loop(incoming, on_picture));

    let network = loop {
        match media.recv().await {
            // Si el descompresor va lento, el canal se llena y aquí se deja
            // de leer de la red; QUIC frena entonces al equipo que comparte.
            Ok(packet) => {
                if packets.send(packet).await.is_err() {
                    // El hilo terminó: su resultado dice por qué.
                    break Ok(());
                }
            }
            Err(TransportError::Closed) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    // Cerrar el canal hace que el hilo termine.
    drop(packets);
    worker.await.map_err(|_| SessionError::WorkerPanicked)??;
    Ok(network?)
}

fn decode_loop<F>(
    mut incoming: mpsc::Receiver<VideoPacket>,
    mut on_picture: F,
) -> Result<(), SessionError>
where
    F: FnMut(Picture<'_>),
{
    let mut decoder = Decoder::new()?;
    // `blocking_recv` devuelve `None` cuando `watch` termina o se cancela.
    while let Some(packet) = incoming.blocking_recv() {
        let Some(image) = decoder.decode(&packet.data)? else {
            continue;
        };
        let latency = now_micros().saturating_sub(packet.capture_time_micros);
        on_picture(Picture {
            width: image.width,
            height: image.height,
            data: image.data,
            latency: Duration::from_micros(latency),
            compressed_len: packet.data.len(),
        });
    }
    Ok(())
}
