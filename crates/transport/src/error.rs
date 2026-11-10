use disacomp_protocol::ProtocolError;

/// Errores del transporte.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuración TLS no válida: {0}")]
    Tls(#[from] rustls::Error),
    #[error("no se pudo generar el certificado: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("opciones de transporte no válidas: {0}")]
    InvalidOptions(&'static str),
    #[error("no se pudo iniciar la conexión: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("conexión perdida: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("error de protocolo: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("la huella del certificado del host no coincide con la esperada")]
    FingerprintMismatch,
    #[error("versión de protocolo incompatible (local: {local}, remota: {remote})")]
    VersionMismatch { local: u16, remote: u16 },
    #[error("el otro extremo no completó el saludo inicial a tiempo")]
    HandshakeTimeout,
    #[error("se esperaba el saludo inicial y llegó otro mensaje")]
    UnexpectedMessage,
    #[error("el otro extremo cerró el canal de mensajes")]
    Closed,
    #[error("el punto de escucha está cerrado")]
    EndpointClosed,
}

impl TransportError {
    /// Indica si tiene sentido reintentar la conexión más tarde.
    ///
    /// Una caída de red es transitoria; una huella o una versión que no
    /// coinciden seguirán fallando por mucho que se reintente.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Io(_) | Self::Connection(_) | Self::HandshakeTimeout | Self::Closed
        )
    }
}

impl From<quinn::WriteError> for TransportError {
    fn from(error: quinn::WriteError) -> Self {
        match error {
            quinn::WriteError::ConnectionLost(cause) => Self::Connection(cause),
            _ => Self::Closed,
        }
    }
}

impl From<quinn::ReadError> for TransportError {
    fn from(error: quinn::ReadError) -> Self {
        match error {
            quinn::ReadError::ConnectionLost(cause) => Self::Connection(cause),
            _ => Self::Closed,
        }
    }
}
