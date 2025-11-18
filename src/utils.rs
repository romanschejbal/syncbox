use std::sync::atomic::AtomicU64;

pub trait HumanBytes {
    fn to_human_size(self) -> String;
}

impl HumanBytes for u64 {
    fn to_human_size(self) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        const THRESHOLD: u64 = 1024;

        if self == 0 {
            return "0 B".to_string();
        }

        let mut size = self as f64;
        let mut unit_index = 0;

        while size >= THRESHOLD as f64 && unit_index < UNITS.len() - 1 {
            size /= THRESHOLD as f64;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", self, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }
}

impl HumanBytes for &AtomicU64 {
    fn to_human_size(self) -> String {
        self.load(std::sync::atomic::Ordering::SeqCst)
            .to_human_size()
    }
}
