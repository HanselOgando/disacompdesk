//! Subcomando `capture`: herramienta de prueba de la Etapa 2.
//!
//! Captura la pantalla durante unos segundos, muestra qué cambió en cada
//! fotograma y guarda una imagen por segundo.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use disacomp_capture::{FramePacer, list_displays, open};

use crate::bmp;

/// Espera máxima por fotograma, para comprobar a menudo si ya se acabó el tiempo.
const MAX_WAIT: Duration = Duration::from_millis(500);

#[derive(clap::Args)]
pub struct CaptureArgs {
    /// Muestra las pantallas disponibles y termina.
    #[arg(long)]
    list: bool,
    /// Número de la pantalla a capturar (ver --list).
    #[arg(short, long, default_value_t = 0)]
    display: usize,
    /// Máximo de fotogramas por segundo.
    #[arg(short, long, default_value_t = 15)]
    fps: u32,
    /// Duración de la captura en segundos.
    #[arg(short, long, default_value_t = 5)]
    seconds: u64,
    /// Carpeta donde guardar una imagen por segundo.
    #[arg(short, long, default_value = "capturas")]
    out: PathBuf,
}

pub fn run(args: CaptureArgs) -> Result<()> {
    if args.list {
        for display in list_displays()? {
            println!(
                "{}: {} — {}×{}{}",
                display.index,
                display.name,
                display.width,
                display.height,
                if display.primary { " (principal)" } else { "" }
            );
        }
        return Ok(());
    }

    let mut capturer = open(args.display)?;
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("no se pudo crear la carpeta {}", args.out.display()))?;

    let display = capturer.display();
    eprintln!(
        "Capturando {} ({}×{}) durante {} s a {} FPS como máximo…",
        display.name, display.width, display.height, args.seconds, args.fps
    );

    let total = Duration::from_secs(args.seconds);
    let started = Instant::now();
    let mut pacer = FramePacer::new(args.fps, started);

    let mut frames = 0u32;
    let mut saved = 0u32;
    let mut next_save = Duration::ZERO;
    let mut copy_time = Duration::ZERO;
    let mut changed = 0.0f64;

    loop {
        // Primero se espera al siguiente hueco (control de FPS)…
        std::thread::sleep(pacer.schedule(Instant::now()));
        let Some(remaining) = total.checked_sub(started.elapsed()) else {
            break;
        };
        // …y después a que la pantalla cambie.
        let Some(frame) = capturer.next_frame(remaining.min(MAX_WAIT))? else {
            continue;
        };

        let elapsed = started.elapsed();
        frames += 1;
        copy_time += frame.copy_time;
        changed += frame.changed_fraction();
        println!(
            "#{frames:<4} t={:>5.2} s  {}×{}  zonas: {:>3}  cambio: {:>5.1} %  copia: {:>5.1} ms",
            elapsed.as_secs_f64(),
            frame.width,
            frame.height,
            frame.dirty.len(),
            frame.changed_fraction() * 100.0,
            frame.copy_time.as_secs_f64() * 1000.0,
        );

        if elapsed >= next_save {
            let path = args.out.join(format!("captura-{saved:03}.bmp"));
            let image = bmp::encode(frame.width, frame.height, frame.data)?;
            std::fs::write(&path, image)
                .with_context(|| format!("no se pudo guardar {}", path.display()))?;
            saved += 1;
            next_save = Duration::from_secs(elapsed.as_secs() + 1);
        }
    }

    let seconds = started.elapsed().as_secs_f64();
    eprintln!();
    eprintln!(
        "Fotogramas: {frames} en {seconds:.1} s ({:.1} FPS reales)",
        f64::from(frames) / seconds
    );
    if frames > 0 {
        eprintln!(
            "Media por fotograma: {:.1} % de pantalla cambiada, {:.1} ms de copia",
            changed / f64::from(frames) * 100.0,
            copy_time.as_secs_f64() * 1000.0 / f64::from(frames),
        );
    } else {
        eprintln!("La pantalla no cambió durante la captura.");
    }
    eprintln!("Imágenes guardadas: {saved} en {}", args.out.display());
    Ok(())
}
