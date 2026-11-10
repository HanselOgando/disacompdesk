//! Herramienta de prueba de la Etapa 1: dos equipos intercambian líneas de
//! texto por una conexión QUIC cifrada.
//!
//! ```text
//! equipo A>  disacomp-cli listen
//! equipo B>  disacomp-cli connect <ip-de-A>:7000 --fingerprint <huella que muestra A>
//! ```

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use disacomp_protocol::Message;
use disacomp_transport::{
    Backoff, Connection, Fingerprint, Identity, Listener, TransportError, TransportOptions, connect,
};
use tokio::sync::mpsc;
use tracing_subscriber::filter::LevelFilter;

#[derive(Parser)]
#[command(version, about = "Prueba de comunicación cifrada entre dos equipos")]
struct Cli {
    /// Muestra el registro detallado del transporte QUIC.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Espera conexiones de otro equipo (rol de host).
    Listen {
        /// Puerto UDP en el que escuchar.
        #[arg(short, long, default_value_t = 7000)]
        port: u16,
        /// Dirección local en la que escuchar. Por defecto, todas las interfaces.
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
        bind: IpAddr,
    },
    /// Se conecta a un equipo que está escuchando (rol de cliente).
    Connect {
        /// Dirección del host, por ejemplo 192.168.1.20:7000.
        addr: SocketAddr,
        /// Huella que muestra el host al arrancar.
        #[arg(short, long)]
        fingerprint: Fingerprint,
    },
}

/// Por qué terminó una sesión.
enum SessionEnd {
    /// El otro equipo se despidió con `Bye`.
    PeerLeft,
    /// El usuario local pidió salir (Ctrl+C o fin de la entrada).
    LocalQuit,
    /// La conexión se cortó sin despedida.
    Lost(TransportError),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(if cli.verbose {
            LevelFilter::DEBUG
        } else {
            LevelFilter::WARN
        })
        .init();

    let mut input = spawn_stdin_reader();
    match cli.command {
        Command::Listen { port, bind } => listen(SocketAddr::new(bind, port), &mut input).await,
        Command::Connect { addr, fingerprint } => {
            connect_and_retry(addr, fingerprint, &mut input).await
        }
    }
}

async fn listen(addr: SocketAddr, input: &mut mpsc::UnboundedReceiver<String>) -> Result<()> {
    let identity = Identity::generate()?;
    let listener = Listener::bind(addr, &identity, TransportOptions::default())
        .with_context(|| format!("no se pudo escuchar en {addr}"))?;
    let local = listener.local_addr()?;

    eprintln!("Escuchando en {local} (UDP).");
    eprintln!("Huella de este host: {}", identity.fingerprint());
    eprintln!(
        "En el otro equipo: disacomp-cli connect <ip-de-este-equipo>:{} --fingerprint {}",
        local.port(),
        identity.fingerprint()
    );

    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            _ = tokio::signal::ctrl_c() => break,
        };
        let connection = match accepted {
            Ok(connection) => connection,
            Err(TransportError::EndpointClosed) => break,
            Err(error) => {
                eprintln!("Conexión entrante rechazada: {error}");
                continue;
            }
        };

        eprintln!("Conectado con {}.", connection.remote_addr());
        match run_session(connection, input).await {
            SessionEnd::PeerLeft => eprintln!("El cliente cerró la sesión. Esperando otra…"),
            SessionEnd::Lost(error) => eprintln!("Conexión perdida ({error}). Esperando otra…"),
            SessionEnd::LocalQuit => break,
        }
    }

    listener.shutdown().await;
    Ok(())
}

async fn connect_and_retry(
    addr: SocketAddr,
    fingerprint: Fingerprint,
    input: &mut mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let options = TransportOptions::default();
    let mut backoff = Backoff::default();

    loop {
        eprintln!("Conectando con {addr}…");
        let attempt = tokio::select! {
            attempt = connect(addr, fingerprint, &options) => attempt,
            _ = tokio::signal::ctrl_c() => return Ok(()),
        };

        let error = match attempt {
            Ok(connection) => {
                eprintln!("Conectado. Escribe y pulsa Intro para enviar; Ctrl+C para salir.");
                backoff.reset();
                match run_session(connection, input).await {
                    SessionEnd::PeerLeft => {
                        eprintln!("El host cerró la sesión.");
                        return Ok(());
                    }
                    SessionEnd::LocalQuit => return Ok(()),
                    SessionEnd::Lost(error) => error,
                }
            }
            Err(error) => error,
        };

        if !error.is_retryable() {
            bail!(error);
        }
        let delay = backoff.next_delay();
        eprintln!(
            "Sin conexión ({error}). Reintentando en {:.1} s…",
            delay.as_secs_f32()
        );
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

/// Reenvía lo que se escribe al otro equipo y muestra lo que llega, hasta que
/// la sesión termina por cualquiera de los dos lados.
async fn run_session(
    connection: Connection,
    input: &mut mpsc::UnboundedReceiver<String>,
) -> SessionEnd {
    let (mut sender, mut receiver) = connection.split();
    loop {
        // `select!` espera varias cosas a la vez y ejecuta la rama de la
        // primera que ocurra; las demás esperas se cancelan. Por eso solo se
        // esperan aquí operaciones que toleran ser canceladas (ver
        // `MessageReceiver::recv`); el envío se hace ya dentro de la rama.
        tokio::select! {
            received = receiver.recv() => match received {
                Ok(Message::Text(text)) => println!("< {text}"),
                Ok(Message::Bye) => return SessionEnd::PeerLeft,
                Ok(Message::Hello { .. }) => {}
                Err(error) => return SessionEnd::Lost(error),
            },
            line = input.recv() => match line {
                Some(line) => match sender.send(&Message::Text(line)).await {
                    Ok(()) => {}
                    // Un mensaje demasiado grande no rompe la conexión.
                    Err(TransportError::Protocol(error)) => eprintln!("No enviado: {error}"),
                    Err(error) => return SessionEnd::Lost(error),
                },
                None => {
                    sender.close().await;
                    return SessionEnd::LocalQuit;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                sender.close().await;
                return SessionEnd::LocalQuit;
            }
        }
    }
}

/// Lee líneas de la entrada estándar en un hilo aparte.
///
/// Leer de la consola es una operación bloqueante: hacerla en una tarea
/// asíncrona detendría al resto. El hilo entrega cada línea por un canal, y el
/// canal se cierra cuando la entrada termina.
fn spawn_stdin_reader() -> mpsc::UnboundedReceiver<String> {
    let (lines, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}
