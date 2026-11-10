use std::net::SocketAddr;
use std::time::Duration;

use disacomp_protocol::{Message, decode, encode};
use tokio::time::timeout;

use crate::{TransportError, TransportOptions};

/// Código de cierre QUIC: fin normal de la sesión.
const CLOSE_NORMAL: u32 = 0;
/// Código de cierre QUIC: el saludo inicial falló.
const CLOSE_HANDSHAKE_FAILED: u32 = 1;

/// Margen para que el otro extremo confirme los últimos datos antes de cerrar.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);
/// Margen para que el aviso de cierre llegue al otro extremo.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Quién inició la conexión. El cliente abre el stream y el servidor lo acepta.
#[derive(Clone, Copy)]
pub(crate) enum Role {
    Client,
    Server,
}

/// Conexión cifrada y ya saludada con otro equipo.
pub struct Connection {
    sender: MessageSender,
    receiver: MessageReceiver,
}

impl Connection {
    /// Abre el stream de mensajes e intercambia `Hello` con el otro extremo.
    ///
    /// `endpoint` es el socket propio de una conexión de cliente, que debe
    /// vivir tanto como ella; las conexiones entrantes comparten el del
    /// `Listener` y pasan `None`.
    pub(crate) async fn establish(
        quic: quinn::Connection,
        role: Role,
        endpoint: Option<quinn::Endpoint>,
        options: &TransportOptions,
        local_version: u16,
    ) -> Result<Self, TransportError> {
        let handshake = async {
            let (send, recv) = match role {
                Role::Client => quic.open_bi().await?,
                // El stream solo se hace visible cuando el cliente escribe en
                // él, y lo primero que escribe es su `Hello`.
                Role::Server => quic.accept_bi().await?,
            };
            let mut sender = MessageSender {
                stream: send,
                quic: quic.clone(),
                endpoint,
            };
            let mut receiver = MessageReceiver {
                stream: recv,
                buffer: Vec::new(),
            };

            sender
                .send(&Message::Hello {
                    version: local_version,
                })
                .await?;
            let outcome = match receiver.recv().await {
                Ok(Message::Hello { version }) if version == local_version => Ok(()),
                Ok(Message::Hello { version }) => Err(TransportError::VersionMismatch {
                    local: local_version,
                    remote: version,
                }),
                Ok(_) => Err(TransportError::UnexpectedMessage),
                Err(error) => Err(error),
            };
            match outcome {
                Ok(()) => Ok(Self { sender, receiver }),
                Err(error) => {
                    // Se cierra con vaciado para que nuestro `Hello` llegue y
                    // el otro extremo pueda diagnosticar el mismo problema.
                    sender.shutdown(CLOSE_HANDSHAKE_FAILED, None).await;
                    Err(error)
                }
            }
        };

        match timeout(options.handshake_timeout, handshake).await {
            Ok(result) => result,
            Err(_) => {
                quic.close(CLOSE_HANDSHAKE_FAILED.into(), b"handshake timeout");
                Err(TransportError::HandshakeTimeout)
            }
        }
    }

    /// Dirección del otro equipo.
    pub fn remote_addr(&self) -> SocketAddr {
        self.sender.quic.remote_address()
    }

    /// Separa la conexión en sus dos mitades para poder enviar y recibir a la
    /// vez desde tareas distintas.
    pub fn split(self) -> (MessageSender, MessageReceiver) {
        (self.sender, self.receiver)
    }
}

/// Mitad de envío de una [`Connection`].
pub struct MessageSender {
    stream: quinn::SendStream,
    quic: quinn::Connection,
    endpoint: Option<quinn::Endpoint>,
}

impl MessageSender {
    /// Envía un mensaje. Los mensajes llegan completos y en orden.
    ///
    /// No se debe cancelar a medias (por ejemplo dentro de `tokio::select!`):
    /// una trama escrita parcialmente dejaría el stream inservible.
    pub async fn send(&mut self, message: &Message) -> Result<(), TransportError> {
        let frame = encode(message)?;
        self.stream.write_all(&frame).await?;
        Ok(())
    }

    /// Cierra la sesión de forma ordenada avisando al otro extremo con `Bye`.
    pub async fn close(self) {
        self.shutdown(CLOSE_NORMAL, Some(Message::Bye)).await;
    }

    async fn shutdown(mut self, code: u32, farewell: Option<Message>) {
        // Todo es "en el mejor de los casos": si la conexión ya está rota no
        // hay nada que entregar y los errores se ignoran a propósito.
        if let Some(message) = farewell {
            let _ = self.send(&message).await;
        }
        // `close` descarta lo que aún no se ha enviado, así que primero se
        // termina el stream y se espera a que el otro extremo lo confirme.
        if self.stream.finish().is_ok() {
            let _ = timeout(FLUSH_TIMEOUT, self.stream.stopped()).await;
        }
        self.quic.close(code.into(), b"");
        if let Some(endpoint) = self.endpoint {
            let _ = timeout(DRAIN_TIMEOUT, endpoint.wait_idle()).await;
        }
    }
}

/// Mitad de recepción de una [`Connection`].
pub struct MessageReceiver {
    stream: quinn::RecvStream,
    /// Bytes recibidos que todavía no forman una trama completa.
    buffer: Vec<u8>,
}

impl MessageReceiver {
    /// Espera el siguiente mensaje.
    ///
    /// Es seguro cancelarla (por ejemplo dentro de `tokio::select!`): los
    /// bytes ya recibidos se conservan en el búfer interno y la siguiente
    /// llamada continúa donde se quedó.
    pub async fn recv(&mut self) -> Result<Message, TransportError> {
        loop {
            if let Some((message, consumed)) = decode(&self.buffer)? {
                self.buffer.drain(..consumed);
                return Ok(message);
            }
            match self.stream.read_chunk(usize::MAX, true).await? {
                Some(chunk) => self.buffer.extend_from_slice(&chunk.bytes),
                None => return Err(TransportError::Closed),
            }
        }
    }
}
