use std::sync::Arc;
use std::time::Duration;

use crate::TransportError;

/// Tiempos que gobiernan la detección de caídas.
#[derive(Debug, Clone)]
pub struct TransportOptions {
    /// Cada cuánto se envía un paquete "sigo aquí" si no hay otro tráfico.
    pub keep_alive_interval: Duration,
    /// Tiempo sin recibir nada tras el cual la conexión se da por perdida.
    /// Debe ser varias veces mayor que `keep_alive_interval`.
    ///
    /// QUIC nunca espera menos de tres "probe timeouts", que dependen de la
    /// latencia medida. Antes de medirla (al conectar con un host que no
    /// responde) eso son unos 3 s, por pequeño que sea este valor.
    pub idle_timeout: Duration,
    /// Plazo para completar el intercambio de `Hello`.
    pub handshake_timeout: Duration,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            keep_alive_interval: Duration::from_secs(2),
            idle_timeout: Duration::from_secs(10),
            handshake_timeout: Duration::from_secs(5),
        }
    }
}

impl TransportOptions {
    pub(crate) fn quic_config(&self) -> Result<Arc<quinn::TransportConfig>, TransportError> {
        let idle_timeout = quinn::IdleTimeout::try_from(self.idle_timeout)
            .map_err(|_| TransportError::InvalidOptions("idle_timeout demasiado grande"))?;

        let mut config = quinn::TransportConfig::default();
        config
            .keep_alive_interval(Some(self.keep_alive_interval))
            .max_idle_timeout(Some(idle_timeout))
            // Por ahora el protocolo usa un único stream bidireccional; no se
            // le permite al otro extremo abrir nada más.
            .max_concurrent_bidi_streams(1u8.into())
            .max_concurrent_uni_streams(0u8.into());
        Ok(Arc::new(config))
    }
}
