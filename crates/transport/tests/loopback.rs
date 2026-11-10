//! Pruebas de integración: cliente y host reales hablando por 127.0.0.1.

// En pruebas, `unwrap` es la forma normal de fallar; clippy solo lo permite
// por sí solo dentro de funciones `#[test]`, no en las auxiliares.
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use disacomp_protocol::Message;
use disacomp_transport::{
    Backoff, Connection, Identity, Listener, TransportError, TransportOptions, connect,
};
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn any_local_port() -> SocketAddr {
    (Ipv4Addr::LOCALHOST, 0).into()
}

fn text(content: &str) -> Message {
    Message::Text(content.to_owned())
}

/// Levanta un host y conecta un cliente. Devuelve (listener, lado host, lado cliente).
async fn connected_pair(identity: &Identity) -> (Listener, Connection, Connection) {
    let options = TransportOptions::default();
    let listener = Listener::bind(any_local_port(), identity, options.clone()).unwrap();
    let addr = listener.local_addr().unwrap();
    let (server, client) = tokio::join!(
        listener.accept(),
        connect(addr, identity.fingerprint(), &options)
    );
    (listener, server.unwrap(), client.unwrap())
}

#[tokio::test]
async fn exchanges_messages_in_both_directions() {
    let identity = Identity::generate().unwrap();
    let (_listener, server, client) = connected_pair(&identity).await;
    let (mut server_tx, mut server_rx) = server.split();
    let (mut client_tx, mut client_rx) = client.split();

    client_tx.send(&text("hola host")).await.unwrap();
    assert_eq!(server_rx.recv().await.unwrap(), text("hola host"));

    server_tx.send(&text("hola cliente")).await.unwrap();
    assert_eq!(client_rx.recv().await.unwrap(), text("hola cliente"));

    // Varios mensajes seguidos llegan completos y en orden.
    for i in 0..100 {
        client_tx
            .send(&text(&format!("mensaje {i}")))
            .await
            .unwrap();
    }
    for i in 0..100 {
        assert_eq!(
            server_rx.recv().await.unwrap(),
            text(&format!("mensaje {i}"))
        );
    }
}

#[tokio::test]
async fn rejects_host_with_unexpected_fingerprint() {
    let identity = Identity::generate().unwrap();
    let impostor = Identity::generate().unwrap();
    let options = TransportOptions::default();
    let listener = Listener::bind(any_local_port(), &impostor, options.clone()).unwrap();
    let addr = listener.local_addr().unwrap();

    // El cliente espera la huella de `identity`, pero quien responde es `impostor`.
    let accept = tokio::spawn(async move { listener.accept().await });
    let result = connect(addr, identity.fingerprint(), &options).await;

    assert!(matches!(result, Err(TransportError::FingerprintMismatch)));
    assert!(!TransportError::FingerprintMismatch.is_retryable());
    assert!(accept.await.unwrap().is_err());
}

#[tokio::test]
async fn orderly_close_delivers_bye_then_ends() {
    let identity = Identity::generate().unwrap();
    let (_listener, server, client) = connected_pair(&identity).await;
    let (_server_tx, mut server_rx) = server.split();
    let (mut client_tx, _client_rx) = client.split();

    client_tx.send(&text("último")).await.unwrap();
    client_tx.close().await;

    assert_eq!(server_rx.recv().await.unwrap(), text("último"));
    assert_eq!(server_rx.recv().await.unwrap(), Message::Bye);
    assert!(server_rx.recv().await.is_err());
}

#[tokio::test]
async fn host_shutdown_is_detected_as_retryable_loss() {
    let identity = Identity::generate().unwrap();
    let (listener, server, client) = connected_pair(&identity).await;
    let (_client_tx, mut client_rx) = client.split();

    drop(server);
    listener.shutdown().await;

    let error = timeout(TEST_TIMEOUT, client_rx.recv())
        .await
        .expect("la caída debe detectarse sin esperar indefinidamente")
        .unwrap_err();
    assert!(error.is_retryable(), "error inesperado: {error}");
}

#[tokio::test]
async fn client_reconnects_after_host_restart() {
    let identity = Identity::generate().unwrap();
    let options = TransportOptions::default();

    // Primera sesión.
    let (listener, server, client) = connected_pair(&identity).await;
    let addr = listener.local_addr().unwrap();
    let (_client_tx, mut client_rx) = client.split();
    drop(server);
    listener.shutdown().await;
    assert!(client_rx.recv().await.is_err());

    // Con el host caído, un intento de conexión falla con un error reintentable.
    // (Tarda unos 3 s: ver la nota sobre el mínimo en `TransportOptions::idle_timeout`.)
    let quick = TransportOptions {
        idle_timeout: Duration::from_millis(300),
        ..TransportOptions::default()
    };
    let error = timeout(TEST_TIMEOUT, connect(addr, identity.fingerprint(), &quick))
        .await
        .expect("el intento debe rendirse solo")
        .err()
        .expect("no puede conectar con el host caído");
    assert!(error.is_retryable(), "error inesperado: {error}");

    // El host vuelve en el mismo puerto y con la misma identidad.
    let restart = async {
        let listener = loop {
            // El sistema puede tardar un instante en liberar el puerto.
            match Listener::bind(addr, &identity, options.clone()) {
                Ok(listener) => break listener,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        let connection = listener.accept().await.unwrap();
        (listener, connection)
    };

    // El cliente reintenta con backoff hasta que el host responde.
    let reconnect = async {
        let mut backoff = Backoff::new(Duration::from_millis(50), Duration::from_millis(200));
        loop {
            match connect(addr, identity.fingerprint(), &options).await {
                Ok(connection) => break connection,
                Err(error) => {
                    assert!(error.is_retryable(), "error inesperado: {error}");
                    tokio::time::sleep(backoff.next_delay()).await;
                }
            }
        }
    };

    let ((_listener, server), client) =
        timeout(TEST_TIMEOUT, async { tokio::join!(restart, reconnect) })
            .await
            .expect("la reconexión no terminó a tiempo");

    // La sesión nueva funciona.
    let (mut server_tx, _server_rx) = server.split();
    let (_client_tx, mut client_rx) = client.split();
    server_tx.send(&text("de vuelta")).await.unwrap();
    assert_eq!(client_rx.recv().await.unwrap(), text("de vuelta"));
}
