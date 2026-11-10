use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use disacomp_protocol::PROTOCOL_VERSION;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};

use crate::connection::{Connection, Role};
use crate::tls::{self, Fingerprint, Identity};
use crate::{TransportError, TransportOptions};

const NO_QUIC_CIPHER: TransportError =
    TransportError::InvalidOptions("el proveedor TLS no ofrece el cifrado inicial de QUIC");

/// Punto de escucha: acepta conexiones entrantes en un puerto UDP.
pub struct Listener {
    endpoint: quinn::Endpoint,
    options: TransportOptions,
}

impl Listener {
    /// Empieza a escuchar en `addr`. Debe llamarse dentro de un runtime de Tokio.
    pub fn bind(
        addr: SocketAddr,
        identity: &Identity,
        options: TransportOptions,
    ) -> Result<Self, TransportError> {
        let crypto = QuicServerConfig::try_from(tls::server_config(identity)?)
            .map_err(|_| NO_QUIC_CIPHER)?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        config.transport_config(options.quic_config()?);

        let endpoint = quinn::Endpoint::server(config, addr)?;
        Ok(Self { endpoint, options })
    }

    /// Dirección real de escucha (útil cuando se pidió el puerto 0).
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Espera la siguiente conexión entrante y completa el saludo con ella.
    ///
    /// Devuelve [`TransportError::EndpointClosed`] si el listener se cerró;
    /// cualquier otro error afecta solo a esa conexión y se puede seguir
    /// llamando a `accept`.
    ///
    /// Limitación conocida: el saludo se hace aquí dentro, así que un cliente
    /// lento retrasa a los siguientes hasta `handshake_timeout`. Es aceptable
    /// mientras solo haya una sesión a la vez.
    pub async fn accept(&self) -> Result<Connection, TransportError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(TransportError::EndpointClosed)?;
        let quic = incoming.await?;
        Connection::establish(quic, Role::Server, None, &self.options, PROTOCOL_VERSION).await
    }

    /// Cierra todas las conexiones y espera a que el aviso llegue a los clientes.
    pub async fn shutdown(self) {
        self.endpoint.close(0u32.into(), b"");
        self.endpoint.wait_idle().await;
    }
}

/// Se conecta al host que escucha en `addr`, aceptándolo solo si su
/// certificado tiene la huella `fingerprint`.
pub async fn connect(
    addr: SocketAddr,
    fingerprint: Fingerprint,
    options: &TransportOptions,
) -> Result<Connection, TransportError> {
    connect_with_version(addr, fingerprint, options, PROTOCOL_VERSION).await
}

async fn connect_with_version(
    addr: SocketAddr,
    fingerprint: Fingerprint,
    options: &TransportOptions,
    version: u16,
) -> Result<Connection, TransportError> {
    // Puerto 0: el sistema operativo elige uno libre para el socket local.
    let local: SocketAddr = if addr.is_ipv6() {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    };
    let endpoint = quinn::Endpoint::client(local)?;

    let (tls, fingerprint_mismatch) = tls::client_config(fingerprint)?;
    let crypto = QuicClientConfig::try_from(tls).map_err(|_| NO_QUIC_CIPHER)?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(options.quic_config()?);

    let quic = match endpoint.connect_with(config, addr, tls::SERVER_NAME)?.await {
        Ok(quic) => quic,
        Err(_) if fingerprint_mismatch.load(Ordering::Relaxed) => {
            return Err(TransportError::FingerprintMismatch);
        }
        Err(error) => return Err(error.into()),
    };
    Connection::establish(quic, Role::Client, Some(endpoint), options, version).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_incompatible_protocol_version() {
        let identity = Identity::generate().unwrap();
        let options = TransportOptions::default();
        let listener =
            Listener::bind((Ipv4Addr::LOCALHOST, 0).into(), &identity, options.clone()).unwrap();
        let addr = listener.local_addr().unwrap();
        let other_version = PROTOCOL_VERSION + 1;

        let (server, client) = tokio::join!(
            listener.accept(),
            connect_with_version(addr, identity.fingerprint(), &options, other_version),
        );

        assert!(matches!(
            server,
            Err(TransportError::VersionMismatch { local, remote })
                if local == PROTOCOL_VERSION && remote == other_version
        ));
        assert!(matches!(
            client,
            Err(TransportError::VersionMismatch { local, remote })
                if local == other_version && remote == PROTOCOL_VERSION
        ));
    }
}
