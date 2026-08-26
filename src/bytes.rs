//! Sizes, as a person reads them.

/// Render a byte count with the largest unit that keeps it above one.
///
/// Exact below a kilobyte: "4096 B" is a real number a user can check against
/// `ls`, where "4.00 KB" invites rounding into a claim the disk will not honour.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
