//! Process liveness by pid, and by pid plus start time, for boot and read-time
//! reconciliation of stranded rows (`main.rs`), for `dispatch wait`'s orphan
//! check (`wait.rs`) and for the executor's watchdog, which stops once the
//! spawned process is gone.
//!
//! A pid alone cannot tell a row's process from an unrelated process the
//! operating system gave the same pid later. [`alive_since`] adds the check the
//! store's own timestamps allow: a row's owner (or child) was already running
//! when the row's `created_at` (or `started_at`) was written, so a process that
//! started later than that is a replacement. The check is a heuristic: a
//! replacement that starts within [`START_ALLOWANCE`] of the recorded time, or
//! a wall clock corrected between the write and the check, is not told apart.

use std::time::{Duration, SystemTime};

/// How much later than a row's recorded time a process may have started and
/// still be that row's process. SQLite writes whole seconds, so a process that
/// started during the second the row was written reads up to one second late.
pub const START_ALLOWANCE: Duration = Duration::from_secs(1);

/// Whether process `pid` is running. On Unix, `kill(pid, 0)`: alive and
/// signalable, or alive but owned by someone else (`EPERM`). On Windows, an
/// open-able process whose exit code is still `STILL_ACTIVE`. Elsewhere there
/// is no portable check, so every pid reads as alive and a peer server's tasks
/// are never clobbered.
pub fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    alive(pid)
}

/// When process `pid` started, as UTC wall time: Linux `/proc/<pid>/stat`
/// (field 22 with `btime` from `/proc/stat` and the clock tick rate), macOS
/// `proc_pidinfo` with `PROC_PIDTBSDINFO`, Windows `GetProcessTimes`. `None` on
/// any other platform, for a pid that is not positive, and whenever the
/// operating system refuses or does not answer (another user's process, a
/// process that has just exited).
pub fn process_start_time(pid: i32) -> Option<SystemTime> {
    if pid <= 0 {
        return None;
    }
    start_time(pid)
}

/// Whether `pid` is running and, when both its start time and `recorded_at`
/// are known, started no later than `recorded_at` plus [`START_ALLOWANCE`]. A
/// missing `recorded_at` (a timestamp that is absent or does not parse) and an
/// unreadable start time leave the pid check alone to decide.
pub fn alive_since(pid: i32, recorded_at: Option<SystemTime>) -> bool {
    alive_since_by(pid, recorded_at, process_start_time)
}

/// [`alive_since`] with the start time supplied by `start_of`, so a test can
/// stand in for the operating system. The start time is not read for a pid
/// that is not running, nor without a `recorded_at` to compare it with.
pub fn alive_since_by(
    pid: i32,
    recorded_at: Option<SystemTime>,
    start_of: impl Fn(i32) -> Option<SystemTime>,
) -> bool {
    if !process_alive(pid) {
        return false;
    }
    match recorded_at {
        Some(_) => started_in_time(start_of(pid), recorded_at),
        None => true,
    }
}

/// [`alive_since`] for a pid read from an INTEGER column: a value that is not a
/// pid is not alive.
pub fn row_pid_alive_since(pid: i64, recorded_at: Option<SystemTime>) -> bool {
    i32::try_from(pid).is_ok_and(|p| alive_since(p, recorded_at))
}

/// True unless both times are known and `started` is later than `recorded_at`
/// plus [`START_ALLOWANCE`].
pub fn started_in_time(started: Option<SystemTime>, recorded_at: Option<SystemTime>) -> bool {
    let (Some(started), Some(recorded_at)) = (started, recorded_at) else {
        return true;
    };
    match recorded_at.checked_add(START_ALLOWANCE) {
        Some(latest) => started <= latest,
        None => true,
    }
}

/// Parse a SQLite `datetime('now')` string ("YYYY-MM-DD HH:MM:SS", UTC) into a
/// `SystemTime`, without pulling in a date crate. Returns None on any malformed field.
/// Uses Howard Hinnant's days-from-civil algorithm.
pub fn parse_sqlite_utc(s: &str) -> Option<SystemTime> {
    let (date, time) = s.trim().split_once(' ')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let mo: i64 = d.next()?.parse().ok()?;
    let da: i64 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let se: i64 = t.next()?.parse().ok()?;
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&da)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&se)
    {
        return None;
    }
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = (if yy >= 0 { yy } else { yy - 399 }) / 400;
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12; // Mar=0 … Feb=11
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se;
    if secs < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64))
}

#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // kill(pid, 0): 0 => alive & signalable; EPERM => alive but not ours; ESRCH => dead.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn alive(pid: i32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code as *mut u32);
        CloseHandle(handle);
        ok != 0 && code as i32 == STILL_ACTIVE
    }
}

#[cfg(not(any(unix, windows)))]
fn alive(_pid: i32) -> bool {
    true
}

#[cfg(target_os = "linux")]
fn start_time(pid: i32) -> Option<SystemTime> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks = stat_start_ticks(&stat)?;
    let boot = boot_time_secs(&std::fs::read_to_string("/proc/stat").ok()?)?;
    // SAFETY: sysconf has no preconditions; it returns -1 for an unknown name.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    started_after_boot(boot, ticks, u64::try_from(hz).ok()?)
}

#[cfg(target_os = "macos")]
fn start_time(pid: i32) -> Option<SystemTime> {
    // SAFETY: `info` is plain old data that proc_pidinfo fills, and the size
    // passed is the size of the buffer.
    let (info, written) = unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let written = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
            size,
        );
        (info, written == size)
    };
    if !written {
        return None;
    }
    let since_epoch = Duration::from_secs(info.pbi_start_tvsec)
        .checked_add(Duration::from_micros(info.pbi_start_tvusec))?;
    SystemTime::UNIX_EPOCH.checked_add(since_epoch)
}

#[cfg(windows)]
fn start_time(pid: i32) -> Option<SystemTime> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the handle is checked before use and closed after; the four
    // FILETIME out-parameters are valid for writes for the whole call.
    let ok = unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if handle.is_null() {
            return None;
        }
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(handle);
        ok
    };
    if ok == 0 {
        return None;
    }
    filetime_to_system_time(
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn start_time(_pid: i32) -> Option<SystemTime> {
    None
}

/// Field 22 of a `/proc/<pid>/stat` line, `starttime`: clock ticks since boot.
///
/// The second field, the command name, is in parentheses and may itself hold
/// spaces and parentheses, so the fields are counted after the last `)`: the
/// text after it starts at field 3, so field 22 is the 20th.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stat_start_ticks(stat: &str) -> Option<u64> {
    let after_comm = &stat[stat.rfind(')')? + 1..];
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

/// The `btime` line of `/proc/stat`: the boot time in seconds since the epoch.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn boot_time_secs(proc_stat: &str) -> Option<u64> {
    proc_stat
        .lines()
        .find_map(|line| line.strip_prefix("btime ")?.trim().parse().ok())
}

/// The wall time `ticks` clock ticks (at `hz` per second) after the boot at
/// `boot_secs`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn started_after_boot(boot_secs: u64, ticks: u64, hz: u64) -> Option<SystemTime> {
    if hz == 0 {
        return None;
    }
    let secs = boot_secs.checked_add(ticks / hz)?;
    let nanos = (ticks % hz).checked_mul(1_000_000_000)? / hz;
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(secs, u32::try_from(nanos).ok()?))
}

/// A Windows `FILETIME` (100 ns intervals since 1601-01-01 UTC) as a `SystemTime`.
#[cfg_attr(not(windows), allow(dead_code))]
fn filetime_to_system_time(intervals: u64) -> Option<SystemTime> {
    /// 1601-01-01 to 1970-01-01, in 100 ns intervals.
    const UNIX_EPOCH_INTERVALS: u64 = 116_444_736_000_000_000;
    let since_unix = intervals.checked_sub(UNIX_EPOCH_INTERVALS)?;
    let nanos = (since_unix % 10_000_000) * 100;
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(since_unix / 10_000_000, nanos as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> i32 {
        std::process::id() as i32
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn this_process_is_alive_and_nonpositive_pids_are_not() {
        assert!(process_alive(me()));
        assert!(!process_alive(0));
        assert!(!process_alive(-1));
    }

    #[test]
    fn the_reader_gives_this_process_a_start_time_in_the_past() {
        let started = process_start_time(me()).unwrap_or_else(|| {
            panic!("no start time for this process on {}", std::env::consts::OS)
        });
        let now = SystemTime::now();
        assert!(
            started <= now + START_ALLOWANCE,
            "{started:?} is after {now:?}"
        );
        assert!(
            started > at(1_000_000_000),
            "{started:?} is implausibly early"
        );
        assert_eq!(process_start_time(0), None);
        assert_eq!(process_start_time(-5), None);
    }

    #[test]
    fn this_process_is_alive_for_a_row_written_after_it_started() {
        assert!(alive_since(me(), Some(SystemTime::now())));
        assert!(alive_since(me(), None), "no recorded time: the pid decides");
    }

    #[test]
    fn this_process_is_dead_for_a_row_written_long_before_it_started() {
        // A row from 2000 cannot belong to a process running now.
        assert!(!alive_since(me(), Some(at(946_684_800))));
    }

    #[test]
    fn a_pid_that_is_not_running_is_dead_whatever_the_times_say() {
        let any = |_| Some(at(1));
        assert!(!alive_since_by(0, Some(at(100)), any));
        assert!(!alive_since_by(-3, None, any));
        assert!(!row_pid_alive_since(i64::from(i32::MAX) + 1, Some(at(100))));
    }

    #[test]
    fn a_process_that_started_after_the_row_plus_the_allowance_is_dead() {
        let row = at(1_800_000_000);
        let started = |secs: u64, extra: Duration| move |_| Some(at(secs) + extra);
        // Before, at and one second after the row: the row's process.
        assert!(alive_since_by(
            me(),
            Some(row),
            started(1_799_999_000, Duration::ZERO)
        ));
        assert!(alive_since_by(
            me(),
            Some(row),
            started(1_800_000_000, Duration::ZERO)
        ));
        // The one-second boundary: inclusive.
        assert!(alive_since_by(
            me(),
            Some(row),
            started(1_800_000_001, Duration::ZERO)
        ));
        assert!(!alive_since_by(
            me(),
            Some(row),
            started(1_800_000_001, Duration::from_millis(1))
        ));
        assert!(!alive_since_by(
            me(),
            Some(row),
            started(1_800_000_002, Duration::ZERO)
        ));
        assert!(!alive_since_by(
            me(),
            Some(row),
            started(1_900_000_000, Duration::ZERO)
        ));
    }

    #[test]
    fn an_unknown_time_leaves_the_pid_check_alone_to_decide() {
        let late = |_| Some(at(2_000_000_000));
        assert!(alive_since_by(me(), None, late), "no recorded time");
        assert!(
            alive_since_by(me(), Some(at(100)), |_| None),
            "unreadable start time"
        );
        assert!(started_in_time(None, None));
        assert!(started_in_time(Some(at(9)), None));
        assert!(started_in_time(None, Some(at(1))));
        assert!(!started_in_time(Some(at(9)), Some(at(1))));
    }

    #[test]
    fn the_start_time_is_not_read_without_a_recorded_time() {
        let read = std::cell::Cell::new(false);
        let spy = |_| {
            read.set(true);
            Some(at(1))
        };
        assert!(alive_since_by(me(), None, spy));
        assert!(!read.get());
        assert!(alive_since_by(me(), Some(at(1)), spy));
        assert!(read.get());
    }

    #[test]
    fn a_reused_pid_is_told_from_the_original_with_timestamps_as_sqlite_writes_them() {
        // The row text is what `datetime('now')` stores; the replacement
        // process started an hour after it, the original a minute before.
        let created = parse_sqlite_utc("2026-10-14 06:23:41").expect("parses");
        let replacement = move |_| Some(created + Duration::from_secs(3600));
        let original = move |_| Some(created - Duration::from_secs(60));
        assert!(!alive_since_by(me(), Some(created), replacement));
        assert!(alive_since_by(me(), Some(created), original));
    }

    #[test]
    fn a_malformed_or_missing_timestamp_parses_to_none() {
        for bad in ["", "garbage", "2026-10-14", "2026-13-01 00:00:00", "x y"] {
            assert_eq!(parse_sqlite_utc(bad), None, "{bad:?}");
        }
        assert_eq!(
            parse_sqlite_utc("2026-10-14 06:23:41"),
            Some(at(1_791_959_021))
        );
    }

    #[test]
    fn the_stat_line_is_read_past_a_command_name_with_spaces_and_parentheses() {
        // Fields 3 to 21 are fillers; field 22 is 123456.
        let tail = "S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 123456 23 24";
        for comm in ["bash", "a b", "x) y (z", "(((", "a)b"] {
            let line = format!("4242 ({comm}) {tail}");
            assert_eq!(stat_start_ticks(&line), Some(123_456), "{comm}");
        }
        assert_eq!(stat_start_ticks("4242 (bash) S 1 2"), None);
        assert_eq!(stat_start_ticks("no parenthesis"), None);
        assert_eq!(
            stat_start_ticks("1 (a) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 x"),
            None
        );
    }

    #[test]
    fn the_boot_time_and_the_ticks_make_a_wall_time() {
        let stat = "cpu  1 2 3\nintr 5\nbtime 1790000000\nprocesses 7\n";
        assert_eq!(boot_time_secs(stat), Some(1_790_000_000));
        assert_eq!(boot_time_secs("cpu 1\nbtime x\n"), None);
        assert_eq!(boot_time_secs("cpu 1\n"), None);
        // 150 ticks at 100 Hz is 1.5 s.
        assert_eq!(
            started_after_boot(1_790_000_000, 150, 100),
            Some(at(1_790_000_001) + Duration::from_millis(500))
        );
        assert_eq!(started_after_boot(10, 0, 100), Some(at(10)));
        assert_eq!(started_after_boot(10, 5, 0), None);
        assert_eq!(started_after_boot(u64::MAX, 100, 100), None);
    }

    #[test]
    fn a_filetime_is_converted_from_1601_to_the_unix_epoch() {
        assert_eq!(
            filetime_to_system_time(116_444_736_000_000_000),
            Some(at(0))
        );
        assert_eq!(
            filetime_to_system_time(116_444_736_000_000_000 + 10_000_000),
            Some(at(1))
        );
        assert_eq!(
            filetime_to_system_time(116_444_736_000_000_000 + 15_000_000),
            Some(at(1) + Duration::from_millis(500))
        );
        assert_eq!(filetime_to_system_time(0), None, "before the Unix epoch");
    }
}
