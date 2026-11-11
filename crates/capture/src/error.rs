/// Errores de la captura de pantalla.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("la captura de pantalla todavía no está disponible en este sistema operativo")]
    Unsupported,
    #[error("no existe la pantalla número {0}")]
    DisplayNotFound(usize),
    #[error("las pantallas rotadas todavía no están soportadas")]
    RotatedDisplay,
    #[error("la pantalla usa un formato de imagen no soportado")]
    UnsupportedFormat,
    /// El sistema operativo rechazó una operación.
    #[error("{context}: {message}")]
    Os {
        /// Qué se estaba intentando hacer.
        context: &'static str,
        /// Código numérico del sistema (0 si no hay ninguno).
        code: i32,
        message: String,
    },
}
