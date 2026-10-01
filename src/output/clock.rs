//! Ages and clock times as the output prints them.

use std::time::Duration;

/// A rough, human-readable age: "42 seconds", "3 hours", "2 days".
///
/// Only the largest unit is shown; this labels a cache entry, so minutes of
/// precision on a two-day-old snapshot would be noise.
pub fn human_age(age: Duration) -> String {
    let (value, unit) = age_parts(age);
    if value == 1 {
        format!("1 {unit}")
    } else {
        format!("{value} {unit}s")
    }
}

/// The same age as a table cell: "42s ago", "3h ago", "2d ago".
///
/// [`human_age`]'s units, abbreviated: a column of ages has to stay narrow
/// enough that the columns after it are still readable.
pub fn ago(age: Duration) -> String {
    let (value, unit) = age_parts(age);
    format!("{value}{} ago", &unit[..1])
}

/// `HH:MM` of a Unix timestamp on this machine's clock.
///
/// Local time, because the only thing done with one of these is to compare it
/// with the clock in front of the reader: "wait until 15:04" has to mean
/// their 15:04. The offset comes from this machine's own zone file; where
/// there is none to read - Windows keeps its zone somewhere else - the time
/// is UTC, which is the closest this can get without a calendar dependency.
pub fn local_clock(unix: u64) -> String {
    clock_of(unix, local_offset(unix as i64))
}

/// [`local_clock`] to the second, for a screen that refreshes every few.
pub fn local_clock_seconds(unix: u64) -> String {
    let local = (unix as i64)
        .saturating_add(local_offset(unix as i64))
        .max(0) as u64;
    let (_, _, _, hour, minute, second) = crate::backup::utc_parts(local);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// [`local_clock`] with the offset handed in, so the formatting is testable
/// on a machine in any time zone.
pub(super) fn clock_of(unix: u64, offset: i64) -> String {
    let local = (unix as i64).saturating_add(offset).max(0) as u64;
    let (_, _, _, hour, minute, _) = crate::backup::utc_parts(local);
    format!("{hour:02}:{minute:02}")
}

/// This machine's UTC offset in seconds at `unix`, and 0 where nothing here
/// can say what it is.
fn local_offset(unix: i64) -> i64 {
    std::fs::read(LOCALTIME)
        .ok()
        .and_then(|bytes| tzif_offset(&bytes, unix))
        .unwrap_or(0)
}

/// The zone file every Unix keeps the machine's own zone in, as a symlink
/// into the zoneinfo database. Absent on Windows, which is one of the two
/// ways [`local_offset`] ends up with nothing to read.
const LOCALTIME: &str = "/etc/localtime";

/// The UTC offset a TZif file (RFC 8536) gives for `unix`, in seconds.
///
/// Written out rather than taken from a calendar crate, for the same reason
/// [`crate::chapcore::sha256_hex`] is: it is one function, it is only ever
/// used to print a clock time, and the release targets keep the dependency
/// set they have.
///
/// A version 2 or later file carries the whole table twice - once with
/// 32-bit transition times, once with 64-bit ones - and the modern `zic`
/// leaves the first copy empty, so the second block is the one to read.
pub(super) fn tzif_offset(bytes: &[u8], unix: i64) -> Option<i64> {
    /// Magic, version and the reserved bytes, before the six counts.
    const COUNTS_AT: usize = 20;
    /// The whole header: the counts are six 32-bit numbers.
    const HEADER: usize = COUNTS_AT + 6 * 4;

    let u32_at = |at: usize| -> Option<u32> {
        bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
    };
    // `isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt`, in that order.
    let counts = |start: usize| -> Option<[u32; 6]> {
        if bytes.get(start..start + 4)? != b"TZif" {
            return None;
        }
        let mut out = [0u32; 6];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u32_at(start + COUNTS_AT + i * 4)?;
        }
        Some(out)
    };

    // Where the block to read starts, and how wide its transition times are.
    let (start, width) = match *bytes.get(4)? >= b'2' {
        false => (0, 4usize),
        true => {
            let [isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt] = counts(0)?;
            let first = HEADER
                + timecnt as usize * 5
                + typecnt as usize * 6
                + charcnt as usize
                + leapcnt as usize * 8
                + isstdcnt as usize
                + isutcnt as usize;
            (first, 8usize)
        }
    };

    let [_, _, _, timecnt, typecnt, _] = counts(start)?;
    let times = start + HEADER;
    let indices = times + timecnt as usize * width;
    let types = indices + timecnt as usize;

    // The last transition at or before `unix` decides which local time type
    // is in force; before the first one, the file's first type is.
    let mut which = 0usize;
    for i in 0..timecnt as usize {
        let at = times + i * width;
        let when = match width {
            8 => i64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?),
            _ => i64::from(i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?)),
        };
        if when > unix {
            break;
        }
        which = *bytes.get(indices + i)? as usize;
    }
    if which >= typecnt as usize {
        return None;
    }
    let at = types + which * 6;
    Some(i64::from(i32::from_be_bytes(
        bytes.get(at..at + 4)?.try_into().ok()?,
    )))
}

/// The largest whole unit of an age, as `(value, singular unit name)`.
///
/// One place decides where a duration stops being seconds, so [`human_age`]
/// and [`ago`] can never disagree about it.
fn age_parts(age: Duration) -> (u64, &'static str) {
    let secs = age.as_secs();
    match secs {
        0..=59 => (secs, "second"),
        60..=3599 => (secs / 60, "minute"),
        3600..=86_399 => (secs / 3600, "hour"),
        _ => (secs / 86_400, "day"),
    }
}
