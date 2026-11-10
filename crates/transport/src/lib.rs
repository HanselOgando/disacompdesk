//! Transporte cifrado entre dos equipos sobre QUIC.
//!
//! - El host crea una [`Identity`], escucha con [`Listener`] y comunica su
//!   [`Fingerprint`] al cliente.
//! - El cliente llama a [`connect`] con la dirección y esa huella.
//! - Ambos obtienen una [`Connection`] por la que intercambian
//!   [`disacomp_protocol::Message`].
//!
//! QUIC aporta el cifrado (TLS 1.3), la entrega ordenada y la detección de
//! caídas; este crate añade la verificación por huella, el saludo de versión y
//! el troceado del flujo de bytes en mensajes.

mod backoff;
mod connection;
mod endpoint;
mod error;
mod options;
mod tls;

pub use backoff::Backoff;
pub use connection::{Connection, MessageReceiver, MessageSender};
pub use endpoint::{Listener, connect};
pub use error::TransportError;
pub use options::TransportOptions;
pub use tls::{Fingerprint, Identity, InvalidFingerprint};
