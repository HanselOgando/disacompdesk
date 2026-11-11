//! Subcomando `share`: comparte la pantalla de este equipo (Etapa 3).
//!
//! Nadie ve nada sin permiso: cada conexión entrante se queda en espera hasta
//! que la persona sentada en este equipo responde "s" en la terminal.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, Result, bail};
use disacomp_protocol::Message;
use disacomp_session::{ShareSettings, share};
use disacomp_transport::{Connection, Identity, Listener, TransportError, TransportOptions};
use tokio::sync::mpsc;

#[derive(clap::Args)]
pub struct ShareArgs {
    /// Puerto UDP en el que escuchar.
    #[arg(short, long, default_value_t = 7000)]
    port: u16,
    /// Dirección local en la que escuchar. Por defecto, todas las interfaces.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    bind: IpAddr,
    /// Número de la pantalla a compartir (ver `capture --list`).
    #[arg(short, long, default_value_t = 0)]
    display: usize,
    /// Máximo de imágenes por segundo.
    #[arg(short, long, default_value_t = 30)]
    fps: u32,
    /// Caudal del vídeo en kilobits por segundo. Más caudal, más nitidez.
    #[arg(short = 'r', long, default_value_t = 6000)]
    bitrate: u32,
}

/// Qué hacer cuando termina la atención a un equipo.
enum Next {
    /// Seguir esperando a otro.
    WaitForAnother,
    /// Salir del programa.
    Quit,
}

pub async fn run(args: ShareArgs, input: &mut mpsc::UnboundedReceiver<String>) -> Result<()> {
    // Se comprueba la pantalla antes de escuchar, para no descubrir el error
    // cuando ya hay alguien conectado.
    let displays = disacomp_capture::list_displays()?;
    let Some(display) = displays.get(args.display) else {
        bail!(
            "no existe la pantalla {} (hay {})",
            args.display,
            displays.len()
        );
    };

    let addr = SocketAddr::new(args.bind, args.port);
    let identity = Identity::generate()?;
    let listener = Listener::bind(addr, &identity, TransportOptions::default())
        .with_context(|| format!("no se pudo escuchar en {addr}"))?;
    let port = listener.local_addr()?.port();
    let fingerprint = identity.fingerprint();

    eprintln!(
        "Listo para compartir {} ({}×{}) por el puerto {port} (UDP).",
        display.name, display.width, display.height
    );
    eprintln!("Para verla desde otro equipo:");
    eprintln!(
        "  con ventana: disacomp-viewer <ip-de-este-equipo>:{port} --fingerprint {fingerprint}"
    );
    eprintln!(
        "  sin ventana: disacomp-cli receive <ip-de-este-equipo>:{port} --fingerprint {fingerprint}"
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
        match serve(connection, &args, input).await {
            Next::WaitForAnother => eprintln!("Esperando otra conexión…"),
            Next::Quit => break,
        }
    }

    listener.shutdown().await;
    Ok(())
}

/// Atiende a un equipo: pide permiso y, si se concede, le envía la pantalla
/// hasta que alguno de los dos lo deja.
async fn serve(
    connection: Connection,
    args: &ShareArgs,
    input: &mut mpsc::UnboundedReceiver<String>,
) -> Next {
    let peer = connection.remote_addr().ip();
    let media = connection.media();
    let (sender, mut receiver) = connection.split();

    // Se descarta lo que se hubiera tecleado antes de la pregunta: una "s"
    // escrita por adelantado no debe contar como permiso.
    while input.try_recv().is_ok() {}
    eprintln!("¿Permitir que {peer} vea esta pantalla? [s/N]");

    let allowed = loop {
        tokio::select! {
            line = input.recv() => match line {
                Some(line) => break is_yes(&line),
                None => {
                    sender.close().await;
                    return Next::Quit;
                }
            },
            received = receiver.recv() => match received {
                Ok(Message::Bye) | Err(_) => {
                    eprintln!("{peer} se fue antes de recibir respuesta.");
                    return Next::WaitForAnother;
                }
                Ok(_) => {}
            },
            _ = tokio::signal::ctrl_c() => {
                sender.close().await;
                return Next::Quit;
            }
        }
    };
    if !allowed {
        eprintln!("Permiso denegado. No se ha enviado nada.");
        sender.close().await;
        return Next::WaitForAnother;
    }

    // Solo a partir de aquí existe el canal de vídeo.
    let stream = match media.open().await {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("No se pudo iniciar el vídeo: {error}");
            return Next::WaitForAnother;
        }
    };
    let display = args.display;
    let sharing = share(
        move || disacomp_capture::open(display),
        stream,
        ShareSettings {
            fps: args.fps,
            bitrate_kbps: args.bitrate,
        },
    );
    // `select!` necesita poder consultar este futuro varias veces sin
    // moverlo de sitio; `pin!` lo fija en su lugar.
    tokio::pin!(sharing);
    eprintln!("Compartiendo la pantalla con {peer}. Ctrl+C para terminar.");

    loop {
        tokio::select! {
            result = &mut sharing => {
                match result {
                    Ok(()) => eprintln!("La transmisión terminó."),
                    Err(error) => eprintln!("La transmisión se detuvo: {error}"),
                }
                sender.close().await;
                return Next::WaitForAnother;
            }
            received = receiver.recv() => match received {
                Ok(Message::Bye) => {
                    eprintln!("{peer} dejó de mirar.");
                    return Next::WaitForAnother;
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("Conexión perdida ({error}).");
                    return Next::WaitForAnother;
                }
            },
            line = input.recv() => if line.is_none() {
                sender.close().await;
                return Next::Quit;
            },
            _ = tokio::signal::ctrl_c() => {
                sender.close().await;
                return Next::Quit;
            }
        }
    }
}

/// Solo un "sí" explícito concede permiso; cualquier otra cosa, incluido
/// pulsar Intro sin más, lo deniega.
fn is_yes(answer: &str) -> bool {
    matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

#[cfg(test)]
mod tests {
    use super::is_yes;

    #[test]
    fn only_an_explicit_yes_grants_permission() {
        for answer in ["s", "S", " si ", "Sí", "y", "yes"] {
            assert!(is_yes(answer), "{answer:?}");
        }
        for answer in ["", "n", "no", "N", "ss", "claro", "s n"] {
            assert!(!is_yes(answer), "{answer:?}");
        }
    }
}
