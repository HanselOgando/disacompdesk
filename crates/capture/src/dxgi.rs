//! Captura en Windows con DXGI Desktop Duplication.
//!
//! Windows compone el escritorio en la tarjeta gráfica. Esta API permite
//! "duplicar" esa imagen: cada vez que el escritorio cambia entrega una
//! textura con la imagen nueva y la lista de rectángulos que cambiaron.
//!
//! Camino de cada fotograma:
//!
//! 1. `AcquireNextFrame` espera a que haya un cambio y presta la textura.
//! 2. Se leen los rectángulos cambiados y los movidos.
//! 3. La textura vive en la tarjeta gráfica, así que se copia a una textura
//!    "staging" (la única clase que la CPU puede leer) y de ahí a memoria
//!    normal, solo en las zonas que cambiaron.
//! 4. `ReleaseFrame` devuelve la textura a Windows.
//!
//! Este es el único módulo del proyecto con `unsafe`: las funciones de
//! Windows son código C que el compilador de Rust no puede comprobar. Cada
//! bloque lleva un comentario `SAFETY` que explica por qué es correcto.
#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{E_ACCESSDENIED, HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_NOT_FOUND,
    DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTDUPL_MOVE_RECT, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
use windows::core::{Interface, PCWSTR};

use crate::{BYTES_PER_PIXEL, CaptureError, DisplayInfo, Frame, Rect, ScreenCapturer};

/// Pausa máxima antes de reintentar cuando la pantalla no se puede capturar.
const RETRY_PAUSE: Duration = Duration::from_millis(250);

/// Convierte un error de Windows en el nuestro, añadiendo qué se intentaba.
fn os(context: &'static str) -> impl FnOnce(windows::core::Error) -> CaptureError {
    move |error| CaptureError::Os {
        context,
        code: error.code().0,
        message: error.message(),
    }
}

/// Errores que se resuelven solos al cabo de un momento: Windows está
/// mostrando un escritorio protegido (aviso de permisos, pantalla de
/// bloqueo) o cambiando de modo de pantalla.
fn is_temporary(error: &CaptureError) -> bool {
    let CaptureError::Os { code, .. } = error else {
        return false;
    };
    [
        E_ACCESSDENIED,
        DXGI_ERROR_ACCESS_LOST,
        DXGI_ERROR_SESSION_DISCONNECTED,
    ]
    .iter()
    .any(|temporary| temporary.0 == *code)
}

/// Una pantalla junto con la tarjeta gráfica a la que está conectada.
struct Output {
    adapter: IDXGIAdapter1,
    output: IDXGIOutput,
    info: DisplayInfo,
}

/// Recorre todas las tarjetas gráficas y sus pantallas activas.
fn enumerate() -> Result<Vec<Output>, CaptureError> {
    // SAFETY: no tiene precondiciones; devuelve un objeto nuevo o un error.
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(os("no se pudo acceder a DXGI"))?;

    let mut outputs = Vec::new();
    for adapter_index in 0.. {
        // SAFETY: llamada sobre un objeto válido; un índice fuera de rango
        // devuelve DXGI_ERROR_NOT_FOUND, que marca el final de la lista.
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(os("no se pudo listar las tarjetas gráficas")(error)),
        };
        for output_index in 0.. {
            // SAFETY: igual que arriba.
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(os("no se pudo listar las pantallas")(error)),
            };
            // SAFETY: llamada sobre un objeto válido; devuelve una copia de la descripción.
            let desc = unsafe { output.GetDesc() }
                .map_err(os("no se pudo leer la descripción de la pantalla"))?;
            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }

            let area = desc.DesktopCoordinates;
            // Windows puede dar estas coordenadas escaladas (si el texto está
            // al 125 %, por ejemplo), así que la resolución real se pregunta
            // aparte y esto queda solo como respaldo.
            let scaled = (
                (area.right - area.left).max(0) as u32,
                (area.bottom - area.top).max(0) as u32,
            );
            let (width, height) = real_resolution(&desc.DeviceName).unwrap_or(scaled);
            let name_len = desc
                .DeviceName
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(desc.DeviceName.len());

            outputs.push(Output {
                info: DisplayInfo {
                    index: outputs.len(),
                    name: String::from_utf16_lossy(&desc.DeviceName[..name_len]),
                    width,
                    height,
                    // La pantalla principal es siempre la que está en el origen.
                    primary: area.left == 0 && area.top == 0,
                },
                adapter: adapter.clone(),
                output,
            });
        }
    }
    Ok(outputs)
}

/// Resolución en píxeles reales del modo de vídeo actual de una pantalla.
fn real_resolution(device_name: &[u16; 32]) -> Option<(u32, u32)> {
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    // SAFETY: `device_name` es una cadena UTF-16 terminada en cero (así la
    // entrega DXGI en un array de tamaño fijo) y `mode` es una estructura
    // viva con su campo `dmSize` relleno, como exige la función.
    let found = unsafe {
        EnumDisplaySettingsW(
            PCWSTR(device_name.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut mode,
        )
    };
    found
        .as_bool()
        .then_some((mode.dmPelsWidth, mode.dmPelsHeight))
}

pub(crate) fn list_displays() -> Result<Vec<DisplayInfo>, CaptureError> {
    Ok(enumerate()?.into_iter().map(|output| output.info).collect())
}

/// Objetos de Windows necesarios para capturar una pantalla. Si Windows
/// invalida la captura, se descarta entero y se crea otro.
struct Session {
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    /// Textura legible por la CPU, del mismo tamaño que la pantalla.
    staging: ID3D11Texture2D,
    info: DisplayInfo,
}

impl Session {
    fn create(index: usize) -> Result<Self, CaptureError> {
        let Output {
            adapter,
            output,
            mut info,
        } = enumerate()?
            .into_iter()
            .nth(index)
            .ok_or(CaptureError::DisplayNotFound(index))?;

        let mut device = None;
        let mut context = None;
        // SAFETY: los punteros de salida apuntan a variables locales vivas.
        // Al indicar una tarjeta concreta, el tipo de controlador debe ser
        // UNKNOWN y el módulo de software nulo, como aquí.
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(os("no se pudo inicializar Direct3D"))?;
        let missing = || CaptureError::Os {
            context: "no se pudo inicializar Direct3D",
            code: 0,
            message: "Windows no devolvió el dispositivo".to_owned(),
        };
        let device = device.ok_or_else(missing)?;
        let context = context.ok_or_else(missing)?;

        let output: IDXGIOutput1 = output
            .cast()
            .map_err(os("la pantalla no admite duplicación"))?;
        // SAFETY: `device` es un dispositivo válido creado sobre la misma
        // tarjeta gráfica que la pantalla, como exige la función.
        let duplication = unsafe { output.DuplicateOutput(&device) }
            .map_err(os("no se pudo iniciar la captura de la pantalla"))?;

        // SAFETY: llamada sobre un objeto válido; devuelve una copia.
        let desc = unsafe { duplication.GetDesc() };
        if desc.Rotation != DXGI_MODE_ROTATION_IDENTITY
            && desc.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED
        {
            return Err(CaptureError::RotatedDisplay);
        }
        if desc.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(CaptureError::UnsupportedFormat);
        }
        info.width = desc.ModeDesc.Width;
        info.height = desc.ModeDesc.Height;

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: info.width,
            Height: info.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        // SAFETY: la descripción y el puntero de salida son variables locales vivas.
        unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
            .map_err(os("no se pudo reservar memoria para la imagen"))?;
        let staging = staging.ok_or_else(missing)?;

        Ok(Self {
            context,
            duplication,
            staging,
            info,
        })
    }

    /// Añade a `dirty` las zonas que cambiaron en el fotograma actual.
    fn read_changes(&self, metadata_bytes: u32, dirty: &mut Vec<Rect>) -> Result<(), CaptureError> {
        if metadata_bytes == 0 {
            return Ok(());
        }
        let (width, height) = (self.info.width, self.info.height);
        let mut push = |rect: RECT| {
            dirty.extend(Rect::from_edges_clamped(
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                width,
                height,
            ));
        };

        // Windows indica el total de bytes de ambas listas juntas; se reserva
        // ese espacio para cada una y así siempre cabe.
        let capacity = metadata_bytes as usize;

        // Zonas movidas (al desplazar una ventana o hacer scroll). El destino
        // del movimiento es, sencillamente, una zona que ahora es distinta.
        let mut moves = vec![
            DXGI_OUTDUPL_MOVE_RECT::default();
            capacity / size_of::<DXGI_OUTDUPL_MOVE_RECT>() + 1
        ];
        let mut used = 0u32;
        // SAFETY: el búfer tiene exactamente el tamaño en bytes que se indica
        // y `used` es una variable local viva. Solo es válido llamarla entre
        // AcquireNextFrame y ReleaseFrame, que es donde estamos.
        unsafe {
            self.duplication.GetFrameMoveRects(
                size_of_val(moves.as_slice()) as u32,
                moves.as_mut_ptr(),
                &mut used,
            )
        }
        .map_err(os("no se pudieron leer las zonas movidas"))?;
        let count = used as usize / size_of::<DXGI_OUTDUPL_MOVE_RECT>();
        moves
            .iter()
            .take(count)
            .for_each(|moved| push(moved.DestinationRect));

        // Zonas redibujadas.
        let mut rects = vec![RECT::default(); capacity / size_of::<RECT>() + 1];
        let mut used = 0u32;
        // SAFETY: igual que arriba.
        unsafe {
            self.duplication.GetFrameDirtyRects(
                size_of_val(rects.as_slice()) as u32,
                rects.as_mut_ptr(),
                &mut used,
            )
        }
        .map_err(os("no se pudieron leer las zonas cambiadas"))?;
        let count = used as usize / size_of::<RECT>();
        rects.iter().take(count).for_each(|&rect| push(rect));

        Ok(())
    }

    /// Copia a `buffer` las zonas `dirty` de la textura que entregó Windows.
    fn copy_regions(
        &self,
        resource: &IDXGIResource,
        dirty: &[Rect],
        buffer: &mut [u8],
    ) -> Result<(), CaptureError> {
        let texture: ID3D11Texture2D = resource
            .cast()
            .map_err(os("la imagen recibida no es una textura"))?;
        // SAFETY: ambas texturas pertenecen al mismo dispositivo y tienen el
        // mismo tamaño y formato (la de destino se creó a partir de la
        // descripción de la duplicación).
        unsafe { self.context.CopyResource(&self.staging, &texture) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: `staging` se creó con acceso de lectura para la CPU y
        // `mapped` es una variable local viva.
        unsafe {
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(os("no se pudo leer la imagen"))?;

        let width = self.info.width as usize;
        let row_bytes = width * BYTES_PER_PIXEL;
        // Cada fila de la textura puede llevar relleno al final: `RowPitch`
        // es la distancia real entre filas, que puede ser mayor que `row_bytes`.
        let row_pitch = mapped.RowPitch as usize;
        let source = mapped.pData as *const u8;
        let usable = !source.is_null() && row_pitch >= row_bytes;

        if usable {
            for rect in dirty {
                let offset = rect.x as usize * BYTES_PER_PIXEL;
                let len = rect.width as usize * BYTES_PER_PIXEL;
                for row in rect.y as usize..(rect.y + rect.height) as usize {
                    // SAFETY: `Map` tuvo éxito, así que `source` apunta a
                    // `alto * row_pitch` bytes legibles hasta `Unmap`. Los
                    // rectángulos están recortados al tamaño de la pantalla
                    // (`Rect::from_edges_clamped`), por lo que `row < alto` y
                    // `offset + len <= row_bytes <= row_pitch`: el tramo cae
                    // dentro de la fila `row`.
                    let pixels = unsafe {
                        std::slice::from_raw_parts(source.add(row * row_pitch + offset), len)
                    };
                    buffer[row * row_bytes + offset..][..len].copy_from_slice(pixels);
                }
            }
        }

        // SAFETY: se deshace el `Map` anterior; `pixels` ya no se usa.
        unsafe { self.context.Unmap(&self.staging, 0) };

        if usable {
            Ok(())
        } else {
            Err(CaptureError::Os {
                context: "no se pudo leer la imagen",
                code: 0,
                message: "Windows entregó una imagen con un formato inesperado".to_owned(),
            })
        }
    }
}

/// Devuelve el fotograma a Windows al salir de ámbito, también si la función
/// termina antes de tiempo por un error. Es el patrón RAII: liberar el
/// recurso en `Drop` para que sea imposible olvidarlo.
struct ReleaseOnDrop<'a>(&'a IDXGIOutputDuplication);

impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        // SAFETY: solo se construye tras un `AcquireNextFrame` con éxito.
        // Si falla no hay nada que hacer: la siguiente adquisición lo dirá.
        let _ = unsafe { self.0.ReleaseFrame() };
    }
}

/// Capturador de una pantalla de Windows.
pub(crate) struct DxgiCapturer {
    /// `None` mientras Windows no deja capturar; se recrea al volver a pedir.
    session: Option<Session>,
    info: DisplayInfo,
    /// Última imagen completa. Se reutiliza entre fotogramas.
    buffer: Vec<u8>,
    dirty: Vec<Rect>,
    /// El siguiente fotograma debe copiarse entero (el primero, o tras
    /// recrear la sesión).
    needs_full_frame: bool,
}

impl DxgiCapturer {
    pub(crate) fn open(index: usize) -> Result<Self, CaptureError> {
        let session = Session::create(index)?;
        Ok(Self {
            info: session.info.clone(),
            session: Some(session),
            buffer: Vec::new(),
            dirty: Vec::new(),
            needs_full_frame: true,
        })
    }
}

impl ScreenCapturer for DxgiCapturer {
    fn display(&self) -> &DisplayInfo {
        &self.info
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame<'_>>, CaptureError> {
        let session = match &self.session {
            Some(session) => session,
            None => match Session::create(self.info.index) {
                Ok(session) => {
                    self.info = session.info.clone();
                    self.needs_full_frame = true;
                    self.session.insert(session)
                }
                Err(error) if is_temporary(&error) => {
                    std::thread::sleep(timeout.min(RETRY_PAUSE));
                    return Ok(None);
                }
                Err(error) => return Err(error),
            },
        };

        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: ambos punteros de salida apuntan a variables locales vivas.
        let acquired = unsafe {
            session
                .duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        };
        if let Err(error) = acquired {
            if error.code() == DXGI_ERROR_WAIT_TIMEOUT {
                return Ok(None);
            }
            if error.code() == DXGI_ERROR_ACCESS_LOST {
                // Cambió la resolución o apareció un escritorio protegido:
                // esta sesión ya no sirve. La próxima llamada crea otra.
                self.session = None;
                return Ok(None);
            }
            return Err(os("no se pudo obtener la imagen de la pantalla")(error));
        }
        let _release = ReleaseOnDrop(&session.duplication);

        // Un tiempo de presentación a cero significa que la imagen no cambió:
        // solo se movió el ratón, que Windows entrega por separado.
        if frame_info.LastPresentTime == 0 {
            return Ok(None);
        }

        let started = Instant::now();
        let (width, height) = (session.info.width, session.info.height);
        self.dirty.clear();
        if self.needs_full_frame {
            self.dirty.push(Rect::full(width, height));
            self.buffer.clear();
            self.buffer
                .resize(width as usize * height as usize * BYTES_PER_PIXEL, 0);
        } else {
            session.read_changes(frame_info.TotalMetadataBufferSize, &mut self.dirty)?;
            if self.dirty.is_empty() {
                return Ok(None);
            }
        }

        let resource = resource.ok_or(CaptureError::Os {
            context: "no se pudo obtener la imagen de la pantalla",
            code: 0,
            message: "Windows no entregó la imagen".to_owned(),
        })?;
        session.copy_regions(&resource, &self.dirty, &mut self.buffer)?;
        self.needs_full_frame = false;

        Ok(Some(Frame {
            width,
            height,
            data: &self.buffer,
            dirty: &self.dirty,
            copy_time: started.elapsed(),
        }))
    }
}
