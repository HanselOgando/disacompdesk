use std::time::{Duration, Instant};

/// Resumen de un tramo de vídeo recibido.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Imágenes por segundo.
    pub fps: f64,
    /// Caudal del vídeo comprimido, en kilobits por segundo.
    pub kbps: f64,
    /// Latencia media, en milisegundos.
    pub latency_ms: f64,
}

/// Acumula lo recibido y entrega un [`Stats`] cada cierto tiempo.
///
/// Como `FramePacer`, recibe la hora como parámetro para poder probarse sin
/// esperas reales.
#[derive(Debug)]
pub struct StatsWindow {
    period: Duration,
    started: Instant,
    pictures: u32,
    bytes: usize,
    latency: Duration,
}

impl StatsWindow {
    pub fn new(period: Duration, now: Instant) -> Self {
        Self {
            period,
            started: now,
            pictures: 0,
            bytes: 0,
            latency: Duration::ZERO,
        }
    }

    /// Anota una imagen recibida. Devuelve el resumen del tramo cuando este
    /// se completa, y empieza uno nuevo.
    pub fn record(
        &mut self,
        compressed_len: usize,
        latency: Duration,
        now: Instant,
    ) -> Option<Stats> {
        self.pictures += 1;
        self.bytes += compressed_len;
        self.latency += latency;

        let elapsed = now.saturating_duration_since(self.started);
        if elapsed < self.period {
            return None;
        }
        let seconds = elapsed.as_secs_f64();
        let stats = Stats {
            fps: f64::from(self.pictures) / seconds,
            kbps: self.bytes as f64 * 8.0 / 1000.0 / seconds,
            latency_ms: self.latency.as_secs_f64() * 1000.0 / f64::from(self.pictures),
        };
        *self = Self::new(self.period, now);
        Some(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_once_per_period_and_starts_over() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut window = StatsWindow::new(Duration::from_secs(1), t0);

        assert_eq!(window.record(1000, ms(10), t0 + ms(250)), None);
        assert_eq!(window.record(1000, ms(20), t0 + ms(500)), None);
        let stats = window.record(500, ms(30), t0 + ms(1000));
        assert_eq!(
            stats,
            Some(Stats {
                fps: 3.0,
                kbps: 20.0,
                latency_ms: 20.0
            })
        );

        // El tramo siguiente no arrastra nada del anterior.
        assert_eq!(window.record(1000, ms(10), t0 + ms(1500)), None);
        let stats = window.record(1000, ms(10), t0 + ms(2000));
        assert_eq!(
            stats,
            Some(Stats {
                fps: 2.0,
                kbps: 16.0,
                latency_ms: 10.0
            })
        );
    }
}
