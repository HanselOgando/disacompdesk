//! Captura de pantalla con detección de zonas cambiadas.
//!
//! La API es la misma en todos los sistemas; lo que cambia es la
//! implementación que hay detrás:
//!
//! - Windows: DXGI Desktop Duplication (módulo `dxgi`).
//! - Resto: aún sin implementar; las funciones devuelven
//!   [`CaptureError::Unsupported`].
//!
//! Uso típico:
//!
//! ```no_run
//! use std::time::Duration;
//!
//! let mut capturer = disacomp_capture::open(0)?;
//! if let Some(frame) = capturer.next_frame(Duration::from_millis(500))? {
//!     println!("{}×{}, {} zonas cambiadas", frame.width, frame.height, frame.dirty.len());
//! }
//! # Ok::<(), disacomp_capture::CaptureError>(())
//! ```
//!
//! La captura es bloqueante y no usa Tokio: está pensada para ejecutarse en
//! su propio hilo.

use std::time::Duration;

#[cfg(windows)]
mod dxgi;
mod error;
mod frame;
mod pacer;

pub use error::CaptureError;
pub use frame::{BYTES_PER_PIXEL, DisplayInfo, Frame, Rect};
pub use pacer::FramePacer;

/// Algo capaz de entregar imágenes de una pantalla.
///
/// Cada sistema operativo aporta su propia implementación; el resto del
/// programa solo conoce este trait.
pub trait ScreenCapturer {
    /// Pantalla que se está capturando. La resolución puede cambiar durante
    /// la captura; cada [`Frame`] lleva la suya.
    fn display(&self) -> &DisplayInfo;

    /// Espera como mucho `timeout` a que la pantalla cambie.
    ///
    /// - `Ok(Some(frame))`: hay imagen nueva.
    /// - `Ok(None)`: no cambió nada en ese tiempo, o la pantalla no se puede
    ///   capturar ahora mismo (por ejemplo, durante un aviso de permisos de
    ///   Windows). Basta con volver a llamar.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame<'_>>, CaptureError>;
}

/// Lista las pantallas conectadas.
pub fn list_displays() -> Result<Vec<DisplayInfo>, CaptureError> {
    #[cfg(windows)]
    {
        dxgi::list_displays()
    }
    #[cfg(not(windows))]
    {
        Err(CaptureError::Unsupported)
    }
}

/// Empieza a capturar la pantalla número `index` de [`list_displays`].
///
/// El capturador debe usarse en el mismo hilo en que se creó.
pub fn open(index: usize) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
    #[cfg(windows)]
    {
        Ok(Box::new(dxgi::DxgiCapturer::open(index)?))
    }
    #[cfg(not(windows))]
    {
        let _ = index;
        Err(CaptureError::Unsupported)
    }
}
