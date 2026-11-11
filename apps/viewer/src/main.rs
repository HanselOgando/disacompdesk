//! Ventana de pruebas de la Etapa 3: muestra la pantalla que comparte otro
//! equipo con `disacomp-cli share`.
//!
//! ```text
//! disacomp-viewer <ip>:7000 --fingerprint <huella que muestra el otro equipo>
//! ```
//!
//! El programa tiene dos mitades que casi no se conocen:
//!
//! - El **hilo principal** lleva la ventana. Los sistemas operativos exigen
//!   que la ventana se maneje desde él, y su bucle de eventos no es
//!   asíncrono.
//! - Un **hilo de red** ejecuta Tokio: conecta, recibe el vídeo y lo
//!   descomprime.
//!
//! Se comunican de tres formas: la última imagen recibida vive en memoria
//! compartida ([`Shared`]); el hilo de red despierta a la ventana con eventos
//! ([`UserEvent`]); y la ventana avisa al hilo de red de que hay que cerrar
//! por un canal de un solo uso.

mod scale;

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use clap::Parser;
use disacomp_protocol::Message;
use disacomp_session::{StatsWindow, watch};
use disacomp_transport::{Fingerprint, TransportOptions, connect};
use tokio::sync::oneshot;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

const TITLE: &str = "Disacomp";

#[derive(Parser)]
#[command(
    version,
    about = "Muestra en una ventana la pantalla que comparte otro equipo"
)]
struct Args {
    /// Dirección del equipo que comparte, por ejemplo 192.168.1.20:7000.
    addr: SocketAddr,
    /// Huella que muestra el equipo que comparte al arrancar.
    #[arg(short, long)]
    fingerprint: Fingerprint,
}

/// Última imagen recibida, en BGRA.
#[derive(Default)]
struct Latest {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// La imagen la escribe el hilo de descompresión y la lee el de la ventana.
/// `Mutex` garantiza que solo uno la toca a la vez; `Arc` permite que los dos
/// hilos sean dueños del mismo dato.
type Shared = Arc<Mutex<Latest>>;

/// Si un hilo falla con el `Mutex` cogido, este queda "envenenado". La imagen
/// sigue siendo utilizable (como mucho, a medio copiar), así que se ignora.
fn lock(shared: &Shared) -> MutexGuard<'_, Latest> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Avisos del hilo de red a la ventana.
enum UserEvent {
    /// Hay una imagen nueva en [`Shared`].
    Picture,
    /// Texto para el título de la ventana.
    Status(String),
    /// La sesión terminó; el texto explica por qué.
    Ended(String),
}

fn main() -> Result<()> {
    let args = Args::parse();

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let shared = Shared::default();
    let (quit, quit_requested) = oneshot::channel();

    let network = std::thread::spawn({
        let proxy = event_loop.create_proxy();
        let shared = Arc::clone(&shared);
        move || {
            let ending = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime
                    .block_on(session(args, shared, proxy.clone(), quit_requested))
                    .unwrap_or_else(|error| format!("Error: {error:#}")),
                Err(error) => format!("Error: {error}"),
            };
            // Si la ventana ya se cerró, no queda nadie a quien avisar.
            let _ = proxy.send_event(UserEvent::Ended(ending));
        }
    });

    let mut app = App {
        shared,
        graphics: None,
    };
    event_loop.run_app(&mut app)?;

    // La ventana se cerró: se avisa al hilo de red y se le espera, para que
    // le dé tiempo a despedirse del otro equipo.
    let _ = quit.send(());
    let _ = network.join();
    Ok(())
}

/// Todo lo que ocurre en la red, de principio a fin. Devuelve el motivo por
/// el que terminó la sesión.
async fn session(
    args: Args,
    shared: Shared,
    proxy: EventLoopProxy<UserEvent>,
    mut quit: oneshot::Receiver<()>,
) -> Result<String> {
    let status = |text: String| {
        let _ = proxy.send_event(UserEvent::Status(text));
    };

    status(format!("conectando con {}…", args.addr));
    let options = TransportOptions::default();
    let connection = tokio::select! {
        connection = connect(args.addr, args.fingerprint, &options) => {
            connection.context("no se pudo conectar")?
        }
        _ = &mut quit => return Ok(String::new()),
    };
    let media = connection.media();
    let (sender, mut receiver) = connection.split();

    status("esperando a que el otro equipo dé permiso…".to_owned());
    // El canal de vídeo solo aparece cuando el otro equipo acepta.
    let stream = loop {
        tokio::select! {
            stream = media.accept() => break stream?,
            received = receiver.recv() => match received {
                Ok(Message::Bye) => return Ok("El otro equipo no dio permiso.".to_owned()),
                Ok(_) => {}
                Err(error) => return Err(error).context("conexión perdida"),
            },
            _ = &mut quit => {
                sender.close().await;
                return Ok(String::new());
            }
        }
    };

    let mut stats = StatsWindow::new(Duration::from_secs(1), Instant::now());
    let picture_proxy = proxy.clone();
    // Se ejecuta en el hilo de descompresión con cada imagen.
    let watching = watch(stream, move |picture| {
        {
            let mut latest = lock(&shared);
            latest.width = picture.width;
            latest.height = picture.height;
            // Reutiliza la memoria de la imagen anterior en lugar de pedir
            // 8 MB nuevos cada vez.
            latest.pixels.clear();
            latest.pixels.extend_from_slice(picture.data);
        }
        let _ = picture_proxy.send_event(UserEvent::Picture);

        if let Some(stats) = stats.record(picture.compressed_len, picture.latency, Instant::now()) {
            let _ = picture_proxy.send_event(UserEvent::Status(format!(
                "{}×{} · {:.0} FPS · {:.0} kbit/s · latencia {:.0} ms",
                picture.width, picture.height, stats.fps, stats.kbps, stats.latency_ms
            )));
        }
    });
    tokio::pin!(watching);

    loop {
        tokio::select! {
            result = &mut watching => {
                result?;
                sender.close().await;
                return Ok("El otro equipo dejó de compartir.".to_owned());
            }
            received = receiver.recv() => match received {
                Ok(Message::Bye) => return Ok("El otro equipo cerró la sesión.".to_owned()),
                Ok(_) => {}
                Err(error) => return Err(error).context("conexión perdida"),
            },
            _ = &mut quit => {
                sender.close().await;
                return Ok(String::new());
            }
        }
    }
}

/// La ventana y la superficie donde se pintan los píxeles.
struct Graphics {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

struct App {
    shared: Shared,
    /// `None` hasta que el sistema permite crear la ventana.
    graphics: Option<Graphics>,
}

impl App {
    fn create_window(event_loop: &ActiveEventLoop) -> Result<Graphics> {
        let attributes = Window::default_attributes()
            .with_title(TITLE)
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
        let window = Rc::new(event_loop.create_window(attributes)?);
        // Los errores de `softbuffer` no se pueden mover entre hilos, cosa
        // que `anyhow` exige, así que se convierten a texto.
        let context = softbuffer::Context::new(Rc::clone(&window))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let surface = softbuffer::Surface::new(&context, Rc::clone(&window))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(Graphics { window, surface })
    }

    fn draw(&mut self) -> Result<(), softbuffer::SoftBufferError> {
        let Some(graphics) = &mut self.graphics else {
            return Ok(());
        };
        let size = graphics.window.inner_size();
        // Una ventana minimizada mide 0×0: no hay nada que pintar.
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        graphics.surface.resize(width, height)?;
        let mut buffer = graphics.surface.buffer_mut()?;
        {
            let latest = lock(&self.shared);
            scale::blit(
                &latest.pixels,
                latest.width as usize,
                latest.height as usize,
                &mut buffer,
                size.width as usize,
                size.height as usize,
            );
        }
        buffer.present()
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        match Self::create_window(event_loop) {
            Ok(graphics) => self.graphics = Some(graphics),
            Err(error) => {
                eprintln!("No se pudo crear la ventana: {error:#}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Picture => {
                if let Some(graphics) = &self.graphics {
                    graphics.window.request_redraw();
                }
            }
            UserEvent::Status(text) => {
                if let Some(graphics) = &self.graphics {
                    graphics.window.set_title(&format!("{TITLE} — {text}"));
                }
            }
            UserEvent::Ended(reason) => {
                if !reason.is_empty() {
                    eprintln!("{reason}");
                }
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => {
                if let Some(graphics) = &self.graphics {
                    graphics.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    eprintln!("No se pudo pintar la ventana: {error}");
                }
            }
            _ => {}
        }
    }
}
