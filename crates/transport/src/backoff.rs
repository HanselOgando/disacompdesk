use std::time::Duration;

/// Espera creciente entre reintentos de conexión (backoff exponencial).
///
/// Cada fallo duplica la espera hasta un máximo, para no saturar la red ni al
/// host mientras está caído; tras un éxito se vuelve al valor inicial.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            next: initial,
        }
    }

    /// Devuelve cuánto esperar antes del siguiente intento.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = self.next.saturating_mul(2).min(self.max);
        delay
    }

    /// Vuelve a la espera inicial. Llamar tras una conexión con éxito.
    pub fn reset(&mut self) {
        self.next = self.initial;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_millis(500), Duration::from_secs(10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_until_max_and_resets() {
        let ms = Duration::from_millis;
        let mut backoff = Backoff::new(ms(100), ms(350));

        assert_eq!(backoff.next_delay(), ms(100));
        assert_eq!(backoff.next_delay(), ms(200));
        assert_eq!(backoff.next_delay(), ms(350));
        assert_eq!(backoff.next_delay(), ms(350));

        backoff.reset();
        assert_eq!(backoff.next_delay(), ms(100));
    }
}
