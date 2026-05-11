// [4] Module pour la gestion reseau avec retry automatique
// [5] Timeout configurable

use std::time::Duration;
use std::thread;

/// Configuration for network operations
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial delay between retries (will be doubled each time)
    pub initial_retry_delay: Duration,
    /// Maximum delay between retries
    pub max_retry_delay: Duration,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Read timeout
    pub read_timeout: Duration,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        NetworkConfig {
            max_retries: 3,
            initial_retry_delay: Duration::from_secs(1),
            max_retry_delay: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(60),
        }
    }
}

/// [4] Execute an operation with exponential backoff retry
pub fn with_retry<T, E, F>(config: &NetworkConfig, operation_name: &str, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    E: std::fmt::Display,
{
    let mut attempts = 0;
    let mut delay = config.initial_retry_delay;

    loop {
        attempts += 1;

        match f() {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempts >= config.max_retries {
                    eprintln!(
                        "  {} failed after {} attempts: {}",
                        operation_name, attempts, e
                    );
                    return Err(e);
                }

                eprintln!(
                    "  {} failed (attempt {}/{}): {}. Retrying in {:?}...",
                    operation_name, attempts, config.max_retries, e, delay
                );

                thread::sleep(delay);

                // Exponential backoff
                delay = std::cmp::min(delay * 2, config.max_retry_delay);
            }
        }
    }
}

/// [3] Simple progress indicator for terminal
pub struct ProgressIndicator {
    total: usize,
    current: usize,
    label: String,
    show_percentage: bool,
    on_progress: Option<Box<dyn Fn(usize, usize, &str) + Send>>,
}

impl ProgressIndicator {
    pub fn new(label: &str, total: usize) -> Self {
        ProgressIndicator {
            total,
            current: 0,
            label: label.to_string(),
            show_percentage: total > 0,
            on_progress: None,
        }
    }

    pub fn with_callback(mut self, cb: Box<dyn Fn(usize, usize, &str) + Send>) -> Self {
        self.on_progress = Some(cb);
        self
    }

    /// Update progress and print status
    pub fn update(&mut self, current: usize) {
        self.current = current;
        self.print();
        if let Some(cb) = &self.on_progress {
            cb(self.current, self.total, &self.label);
        }
    }

    /// Increment by one
    pub fn inc(&mut self) {
        self.current += 1;
        self.print();
        if let Some(cb) = &self.on_progress {
            cb(self.current, self.total, &self.label);
        }
    }

    /// Print current progress
    fn print(&self) {
        if self.show_percentage && self.total > 0 {
            let percentage = (self.current as f64 / self.total as f64 * 100.0) as u32;
            let bar_width = 30;
            let filled = (percentage as usize * bar_width) / 100;
            let empty = bar_width - filled;

            eprint!(
                "\r  {} [{}{}] {}/{} ({}%)",
                self.label,
                "=".repeat(filled),
                " ".repeat(empty),
                self.current,
                self.total,
                percentage
            );
        } else {
            eprint!("\r  {} {}", self.label, self.current);
        }
    }

    /// Finish and print newline
    pub fn finish(&self) {
        if self.show_percentage && self.total > 0 {
            eprintln!(
                "\r  {} [{}] {}/{} (100%)",
                self.label,
                "=".repeat(30),
                self.total,
                self.total
            );
        } else {
            eprintln!("\r  {} {} - Done", self.label, self.current);
        }
    }

    /// Finish with custom message
    pub fn finish_with_message(&self, msg: &str) {
        eprintln!("\r  {} - {}", self.label, msg);
    }
}

/// [3] Spinner for operations with unknown duration
pub struct Spinner {
    frames: Vec<char>,
    current: usize,
    label: String,
}

impl Spinner {
    pub fn new(label: &str) -> Self {
        Spinner {
            frames: vec!['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'],
            current: 0,
            label: label.to_string(),
        }
    }

    /// Tick the spinner
    pub fn tick(&mut self) {
        eprint!("\r  {} {}", self.frames[self.current], self.label);
        self.current = (self.current + 1) % self.frames.len();
    }

    /// Finish with success
    pub fn finish_success(&self, msg: &str) {
        eprintln!("\r  [OK] {} - {}", self.label, msg);
    }

    /// Finish with error
    pub fn finish_error(&self, msg: &str) {
        eprintln!("\r  [ERR] {} - {}", self.label, msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_config_default() {
        let config = NetworkConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.connect_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_progress_indicator() {
        let mut progress = ProgressIndicator::new("Test", 10);
        progress.update(5);
        assert_eq!(progress.current, 5);
    }

    #[test]
    fn test_with_retry_success() {
        let config = NetworkConfig::default();
        let result: Result<i32, &str> = with_retry(&config, "test", || Ok(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_with_retry_failure() {
        let mut config = NetworkConfig::default();
        config.max_retries = 2;
        config.initial_retry_delay = Duration::from_millis(10);

        let mut attempts = 0;
        let result: Result<i32, &str> = with_retry(&config, "test", || {
            attempts += 1;
            Err("always fails")
        });

        assert!(result.is_err());
        assert_eq!(attempts, 2);
    }

    #[test]
    fn test_with_retry_eventual_success() {
        let mut config = NetworkConfig::default();
        config.max_retries = 3;
        config.initial_retry_delay = Duration::from_millis(10);

        let mut attempts = 0;
        let result: Result<i32, &str> = with_retry(&config, "test", || {
            attempts += 1;
            if attempts < 2 {
                Err("temporary failure")
            } else {
                Ok(42)
            }
        });

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 2);
    }
}
