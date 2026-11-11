use std::time::{Duration, Instant};

/// Reparte las capturas en el tiempo para no superar los FPS pedidos.
///
/// Recibe la hora como parámetro en lugar de consultarla: así la lógica se
/// puede probar con horas inventadas, sin esperas reales.
#[derive(Debug, Clone)]
pub struct FramePacer {
    interval: Duration,
    next: Instant,
}

impl FramePacer {
    /// `fps` es el máximo de fotogramas por segundo; 0 se trata como 1.
    pub fn new(fps: u32, now: Instant) -> Self {
        Self {
            interval: Duration::from_secs(1) / fps.max(1),
            next: now,
        }
    }

    /// Reserva el siguiente hueco y devuelve cuánto hay que esperar hasta él.
    ///
    /// Si vamos con retraso no se intenta recuperar con una ráfaga: el
    /// siguiente hueco parte de `now`, de modo que dos huecos nunca quedan
    /// más juntos que el intervalo.
    ///
    /// Lo que se limita es el ritmo de los intentos, y por tanto la media de
    /// FPS. Si un fotograma llega tarde porque la pantalla tardó en cambiar,
    /// el siguiente puede seguirle a menos de un intervalo.
    pub fn schedule(&mut self, now: Instant) -> Duration {
        let start = self.next.max(now);
        self.next = start + self.interval;
        start - now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn first_frame_does_not_wait() {
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(10, t0);
        assert_eq!(pacer.schedule(t0), Duration::ZERO);
    }

    #[test]
    fn keeps_a_steady_interval_when_on_time() {
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(10, t0); // un hueco cada 100 ms
        pacer.schedule(t0);

        // La captura tardó 30 ms: faltan 70 para el siguiente hueco.
        assert_eq!(pacer.schedule(t0 + 30 * MS), 70 * MS);
        // Los huecos no se desplazan: el tercero sigue estando en t0 + 200 ms.
        assert_eq!(pacer.schedule(t0 + 150 * MS), 50 * MS);
    }

    #[test]
    fn does_not_burst_after_falling_behind() {
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(10, t0);
        pacer.schedule(t0);

        // Llegamos medio segundo tarde: se captura ya...
        assert_eq!(pacer.schedule(t0 + 500 * MS), Duration::ZERO);
        // ...pero la siguiente espera el intervalo entero.
        assert_eq!(pacer.schedule(t0 + 500 * MS), 100 * MS);
    }

    #[test]
    fn zero_fps_is_treated_as_one() {
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(0, t0);
        pacer.schedule(t0);
        assert_eq!(pacer.schedule(t0), Duration::from_secs(1));
    }
}
