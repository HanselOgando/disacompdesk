//! Lógica de una sesión de pantalla compartida, independiente del programa
//! que la use (hoy la herramienta de consola y la ventana de pruebas; más
//! adelante la aplicación de escritorio).
//!
//! - [`share`]: captura la pantalla, la comprime y la envía.
//! - [`watch`]: recibe el vídeo, lo descomprime y entrega cada imagen.
//!
//! Las dos mezclan trabajo de dos clases:
//!
//! - **Bloqueante y pesado** (capturar, comprimir, descomprimir): va en un
//!   hilo propio, para no detener al resto.
//! - **De espera** (la red): va en una tarea asíncrona.
//!
//! Ambos lados se comunican por un canal con sitio para un solo paquete. Ese
//! tamaño mínimo es deliberado: si la red o el otro equipo van lentos, el
//! canal se llena y quien produce se queda esperando en lugar de acumular
//! vídeo atrasado. A eso se le llama contrapresión, y es lo que mantiene baja
//! la latencia a costa de imágenes por segundo.

mod share;
mod stats;
mod watch;

pub use share::{ShareSettings, share};
pub use stats::{Stats, StatsWindow};
pub use watch::{Picture, watch};

use std::time::{SystemTime, UNIX_EPOCH};

use disacomp_capture::CaptureError;
use disacomp_codec::CodecError;
use disacomp_transport::TransportError;

/// Errores de una sesión.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("error de captura: {0}")]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("error de red: {0}")]
    Transport(#[from] TransportError),
    #[error("el hilo de vídeo terminó de forma inesperada")]
    WorkerPanicked,
}

/// Hora actual en microsegundos desde 1970. Viaja con cada imagen para que
/// quien la recibe pueda calcular cuánto tardó en llegar.
fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_micros() as u64)
}
