//! Result timestamps (`Time.now` in `Executor.execute`).
//!
//! The source records local times: the instant plus the host's UTC offset
//! at that instant, not flagged as UTC (`2026-11-13 14:51:52 +0500`). The
//! port reads the same offset from the host time zone rules (`localtime_r`
//! on Unix, which honours `TZ`; `SystemTimeToTzSpecificLocalTime` on
//! Windows). A caller can instead pass an explicit
//! [`Clock::System`] offset. Targets without a time zone service use `+0000`.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::runtime::Clock;
use crate::types::value::Time;

/// The current instant in the offset the clock selects. Only an explicit
/// [`Clock::System`] overrides the host offset; [`Clock::Fixed`] fixes the
/// day used by date parsing and does not change timestamps.
pub(super) fn now(clock: Option<Clock>) -> Time {
    let (seconds, nanos) = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (
            i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            d.subsec_nanos(),
        ),
        Err(e) => {
            let d = e.duration();
            let whole = -i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            match d.subsec_nanos() {
                0 => (whole, 0),
                n => (whole - 1, 1_000_000_000 - n),
            }
        }
    };
    let utc_offset = match clock {
        Some(Clock::System { utc_offset_secs }) => utc_offset_secs,
        _ => native_utc_offset(seconds).unwrap_or(0),
    };
    Time::new(seconds, nanos, utc_offset, false)
}

/// The host's UTC offset, in seconds, at a Unix time.
pub fn native_utc_offset(unix_seconds: i64) -> Option<i32> {
    platform::utc_offset(unix_seconds)
}

#[cfg(unix)]
mod platform {
    use std::os::raw::{c_char, c_int, c_long};

    /// `struct tm` as glibc, musl and the BSD/macOS libc lay it out, with
    /// spare room at the end.
    #[repr(C)]
    #[allow(dead_code)]
    struct Tm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
        spare: [u8; 64],
    }

    extern "C" {
        fn tzset();
        fn localtime_r(time: *const c_long, out: *mut Tm) -> *mut Tm;
    }

    pub(super) fn utc_offset(unix_seconds: i64) -> Option<i32> {
        let time = c_long::try_from(unix_seconds).ok()?;
        let mut tm = Tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
            spare: [0; 64],
        };
        // SAFETY: `time` and `tm` are valid for the calls; `localtime_r`
        // writes only into `tm`, which is at least as large as `struct tm`.
        // `tzset` re-reads `TZ`, as Ruby does before `localtime`.
        let result = unsafe {
            tzset();
            localtime_r(&time, &mut tm)
        };
        if result.is_null() {
            return None;
        }
        i32::try_from(tm.tm_gmtoff).ok()
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    #[repr(C)]
    #[allow(dead_code)]
    #[derive(Default, Clone, Copy)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SystemTimeToTzSpecificLocalTime(
            time_zone: *const c_void,
            universal: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }

    pub(super) fn utc_offset(unix_seconds: i64) -> Option<i32> {
        let days = unix_seconds.div_euclid(86_400);
        let second_of_day = unix_seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        let universal = SystemTime {
            year: u16::try_from(year).ok()?,
            month: u16::try_from(month).ok()?,
            day_of_week: 0,
            day: u16::try_from(day).ok()?,
            hour: (second_of_day / 3600) as u16,
            minute: (second_of_day % 3600 / 60) as u16,
            second: (second_of_day % 60) as u16,
            milliseconds: 0,
        };
        let mut local = SystemTime::default();
        // SAFETY: both structures are valid SYSTEMTIME values for the call;
        // a null zone selects the current time zone.
        let ok =
            unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &universal, &mut local) };
        if ok == 0 {
            return None;
        }
        let local_seconds = days_from_civil(
            i64::from(local.year),
            i64::from(local.month),
            i64::from(local.day),
        ) * 86_400
            + i64::from(local.hour) * 3600
            + i64::from(local.minute) * 60
            + i64::from(local.second);
        i32::try_from(local_seconds - unix_seconds).ok()
    }

    /// Howard Hinnant's `civil_from_days`.
    fn civil_from_days(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + i64::from(month <= 2), month, day)
    }

    /// Howard Hinnant's `days_from_civil`.
    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let yoe = year - era * 400;
        let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) fn utc_offset(_unix_seconds: i64) -> Option<i32> {
        None
    }
}
