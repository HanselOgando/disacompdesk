//! Subcomando `receive`: recibe una pantalla compartida sin mostrarla
//! (Etapa 3). Sirve para probar el vídeo en equipos sin entorno gráfico:
//! muestra estadísticas cada segundo y guarda una imagen de vez en cuando.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use disacomp_protocol::Message;
use disacomp_session::{StatsWindow, watch};
use disacomp_transport::{Fingerprint, TransportOptions, connect};

use crate::bmp;

/// Cada cuánto se guarda una imagen.
const SAVE_EVERY: Duration = Duration::from_secs(5);

#[derive(clap::Args)]
pub struct ReceiveArgs {
    /// Dirección del equipo que comparte, por ejemplo 192.168.1.20:7000.
    addr: SocketAddr,
    /// Huella que muestra el equipo que comparte al arrancar.
    #[arg(short, long)]
    fingerprint: Fingerprint,
    /// Carpeta donde guardar una imagen cada pocos segundos.
    #[arg(short, long, default_value = "capturas")]
    out: PathBuf,
}

pub async fn run(args: ReceiveArgs) -> Result<()> {
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("no se pudo crear la carpeta {}", args.out.display()))?;

    eprintln!("Conectando con {}…", args.addr);
    let options = TransportOptions::default();
    let connection = tokio::select! {
        connection = connect(args.addr, args.fingerprint, &options) => connection?,
        _ = tokio::signal::ctrl_c() => return Ok(()),
    };
    let media = connection.media();
    let (sender, mut receiver) = connection.split();

    eprintln!("Conectado. Esperando a que el otro equipo dé permiso…");
    // El canal de vídeo solo aparece cuando el otro equipo acepta.
    let stream = loop {
        tokio::select! {
            stream = media.accept() => break stream?,
            received = receiver.recv() => match received {
                Ok(Message::Bye) => {
                    eprintln!("El otro equipo no dio permiso.");
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) => return Err(error).context("conexión perdida"),
            },
            _ = tokio::signal::ctrl_c() => {
                sender.close().await;
                return Ok(());
            }
        }
    };
    eprintln!("Recibiendo vídeo. Ctrl+C para terminar.");

    let out = args.out;
    let mut stats = StatsWindow::new(Duration::from_secs(1), Instant::now());
    let mut next_save = Instant::now();
    let mut saved = 0u32;
    // Esta función se ejecuta en el hilo de descompresión con cada imagen.
    let watching = watch(stream, move |picture| {
        let now = Instant::now();
        if let Some(stats) = stats.record(picture.compressed_len, picture.latency, now) {
            println!(
                "{}×{}  {:>4.1} FPS  {:>6.0} kbit/s  latencia {:>5.0} ms",
                picture.width, picture.height, stats.fps, stats.kbps, stats.latency_ms
            );
        }
        if now >= next_save {
            next_save = now + SAVE_EVERY;
            let path = out.join(format!("recibida-{saved:03}.bmp"));
            let written = bmp::encode(picture.width, picture.height, picture.data)
                .and_then(|image| Ok(std::fs::write(&path, image)?));
            match written {
                Ok(()) => {
                    saved += 1;
                    eprintln!("Imagen guardada: {}", path.display());
                }
                Err(error) => eprintln!("No se pudo guardar {}: {error}", path.display()),
            }
        }
    });
    tokio::pin!(watching);

    loop {
        tokio::select! {
            result = &mut watching => {
                result?;
                eprintln!("El otro equipo dejó de compartir.");
                sender.close().await;
                return Ok(());
            }
            received = receiver.recv() => match received {
                Ok(Message::Bye) => {
                    eprintln!("El otro equipo cerró la sesión.");
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) => return Err(error).context("conexión perdida"),
            },
            _ = tokio::signal::ctrl_c() => {
                sender.close().await;
                return Ok(());
            }
        }
    }
}
