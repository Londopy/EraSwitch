// log_util.rs
// tiny file logger: timestamped lines in <data dir>\debug.log, never fails the caller

use std::fs::OpenOptions;
use std::io::Write;

// rotate once the log passes ~512 KB, keeping one previous file
const MAX_LOG_BYTES: u64 = 512 * 1024;

pub fn write(msg: &str) {
    let dir = crate::config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("debug.log");

    if std::fs::metadata(&log_path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&log_path, log_path.with_file_name("debug.log.1"));
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let now = chrono::Local::now();
        let _ = writeln!(file, "[{}] {}", now.format("%Y-%m-%d %H:%M:%S%.3f"), msg);
    }
}

// macro so logging looks like eprintln
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::log_util::write(&format!($($arg)*))
    };
}
