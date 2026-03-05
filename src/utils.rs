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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bytes() {
        assert_eq!(0u64.to_human_size(), "0 B");
    }

    #[test]
    fn bytes_below_threshold() {
        assert_eq!(512u64.to_human_size(), "512 B");
        assert_eq!(1u64.to_human_size(), "1 B");
        assert_eq!(1023u64.to_human_size(), "1023 B");
    }

    #[test]
    fn kilobytes() {
        assert_eq!(1024u64.to_human_size(), "1.0 KB");
        assert_eq!(1536u64.to_human_size(), "1.5 KB");
    }

    #[test]
    fn megabytes() {
        assert_eq!(1_048_576u64.to_human_size(), "1.0 MB");
    }

    #[test]
    fn gigabytes() {
        assert_eq!(1_073_741_824u64.to_human_size(), "1.0 GB");
    }

    #[test]
    fn terabytes() {
        assert_eq!(1_099_511_627_776u64.to_human_size(), "1.0 TB");
    }
}
