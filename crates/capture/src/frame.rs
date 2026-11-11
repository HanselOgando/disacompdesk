use std::time::Duration;

/// Bytes por píxel: azul, verde, rojo y un cuarto byte sin usar.
pub const BYTES_PER_PIXEL: usize = 4;

/// Rectángulo en píxeles dentro de una pantalla. El origen es la esquina
/// superior izquierda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// Rectángulo que cubre una pantalla completa.
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Construye un rectángulo a partir de sus bordes (como los da Windows),
    /// recortándolo a una pantalla de `width` × `height`. Devuelve `None` si
    /// queda vacío.
    ///
    /// El recorte importa: estos datos vienen del sistema y se usan después
    /// para copiar memoria, así que nunca deben salirse de la imagen.
    pub fn from_edges_clamped(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let clamp = |value: i32, max: u32| (value.max(0) as u32).min(max);
        let (left, right) = (clamp(left, width), clamp(right, width));
        let (top, bottom) = (clamp(top, height), clamp(bottom, height));
        if left >= right || top >= bottom {
            return None;
        }
        Some(Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }

    /// Superficie en píxeles.
    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Descripción de una pantalla conectada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    /// Posición en la lista de [`crate::list_displays`]; es lo que se pasa a
    /// [`crate::open`].
    pub index: usize,
    /// Nombre que le da el sistema, por ejemplo `\\.\DISPLAY1`.
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Si es la pantalla principal.
    pub primary: bool,
}

/// Imagen de la pantalla en un instante.
///
/// Toma prestada la memoria del capturador (por eso lleva el tiempo de vida
/// `'a`): no se copia la imagen para entregarla, pero hay que terminar de
/// usarla antes de pedir el fotograma siguiente. El compilador lo garantiza.
#[derive(Debug)]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// Píxeles en formato BGRA, fila a fila y sin relleno: `width * height * 4`
    /// bytes. Siempre contiene la imagen completa.
    pub data: &'a [u8],
    /// Zonas que cambiaron desde el fotograma anterior. Pueden solaparse.
    pub dirty: &'a [Rect],
    /// Tiempo que costó traer la imagen de la tarjeta gráfica a memoria.
    pub copy_time: Duration,
}

impl Frame<'_> {
    /// Fracción de la pantalla (0.0 a 1.0) que cambió. Es aproximada: si las
    /// zonas se solapan se cuentan dos veces, por eso se limita a 1.0.
    pub fn changed_fraction(&self) -> f64 {
        let total = u64::from(self.width) * u64::from(self.height);
        if total == 0 {
            return 0.0;
        }
        let changed: u64 = self.dirty.iter().map(Rect::area).sum();
        (changed as f64 / total as f64).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_inside_the_screen_is_kept() {
        let rect = Rect::from_edges_clamped(10, 20, 110, 70, 1920, 1080);
        assert_eq!(
            rect,
            Some(Rect {
                x: 10,
                y: 20,
                width: 100,
                height: 50
            })
        );
    }

    #[test]
    fn rect_is_clamped_to_the_screen() {
        let rect = Rect::from_edges_clamped(-50, -50, 5000, 5000, 1920, 1080);
        assert_eq!(rect, Some(Rect::full(1920, 1080)));
    }

    #[test]
    fn empty_or_outside_rects_are_discarded() {
        assert_eq!(Rect::from_edges_clamped(10, 10, 10, 50, 100, 100), None);
        assert_eq!(Rect::from_edges_clamped(50, 10, 20, 50, 100, 100), None);
        assert_eq!(Rect::from_edges_clamped(200, 200, 300, 300, 100, 100), None);
        assert_eq!(Rect::from_edges_clamped(-30, -30, -10, -10, 100, 100), None);
    }

    #[test]
    fn changed_fraction_is_capped_at_one() {
        let dirty = [Rect::full(100, 100), Rect::full(100, 100)];
        let frame = Frame {
            width: 100,
            height: 100,
            data: &[],
            dirty: &dirty,
            copy_time: Duration::ZERO,
        };
        assert_eq!(frame.changed_fraction(), 1.0);

        let quarter = [Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 50,
        }];
        let frame = Frame {
            dirty: &quarter,
            ..frame
        };
        assert_eq!(frame.changed_fraction(), 0.25);
    }
}
