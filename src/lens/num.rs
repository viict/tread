//! A token count as a row spells it.
//!
//! One function, in the seam and not in a dialect, because the same number
//! appears on a record's row, on the group row over a folded run and in the
//! status bar — and two spellings of it would drift, which is the argument that
//! put `jsonrow` under both JSON sources. Its own module so
//! [`super`] stays under the size limit.
#![deny(unsafe_code)]

/// The scales a token count is spelled at, largest first. `u64::MAX` is about
/// 1.8×10¹⁹, so `E` is the last rung there can be and the widest answer this
/// can give is four columns.
const SCALES: [(u64, char); 6] = [
    (1_000_000_000_000_000_000, 'E'),
    (1_000_000_000_000_000, 'P'),
    (1_000_000_000_000, 'T'),
    (1_000_000_000, 'G'),
    (1_000_000, 'M'),
    (1_000, 'k'),
];

/// A token count in at most **four display columns**: `380`, `1.2k`, `18k`,
/// `1.8M`. A number column is only a column if every number in it is the same
/// width, so the four is a promise and not an estimate.
///
/// # Floored, never rounded
///
/// `999` is `999` and not `1.0k`; `1999` is `1.9k` and not `2.0k`. A row that
/// says `18k` is therefore a promise of *at least* 18,000, which is the reading
/// a person makes of a truncated number and the only one that never overstates
/// what a session spent. The cost is that a bucket hides its magnitude — `18k`
/// is anything up to 18,999, and floored numbers will not add up by eye. The
/// exact integers are one `Enter` away in `detail()`, and the group row and the
/// status bar spell the *exact* sum through this function rather than summing
/// what the rows show.
///
/// It lives in the seam and not in a dialect because a group row spells the
/// same number a record row does, and two spellings would drift — the argument
/// that put `jsonrow` under both JSON sources.
pub fn tokens(n: u64) -> String {
    for (scale, suffix) in SCALES {
        if n < scale {
            continue;
        }
        let whole = n / scale;
        // Two digits leave no room for a decimal, and `18k` is as much as four
        // columns can honestly say.
        if whole >= 10 {
            return format!("{whole}{suffix}");
        }
        return format!("{whole}.{}{suffix}", (n % scale) * 10 / scale);
    }
    format!("{n}")
}
