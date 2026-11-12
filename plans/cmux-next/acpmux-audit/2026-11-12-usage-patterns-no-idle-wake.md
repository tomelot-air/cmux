# Notice to PL-acpmux: the usage-patterns thread no longer wakes an idle acpmux (2026-11-12)

Owner of the change: RUST TIP REDS (a tip-red repair of #40554, which added the crate).

- What was wrong (finding 5, rule D6): `cmux_usage_patterns::install("acpmux")` (`daemon.rs`)
  started a thread `usage-patterns` that slept 5 s and then looked at the settings file, for the
  life of the daemon, also while `diagnostics.usagePatterns` is off (the default). The test
  `acpmux_daemon_with_finished_chats_has_no_periodic_wakeups` (`tests/daemon_idle_chats.rs`) named
  that thread, and the two daemon tests of `cmux-tui/tests/idle_wakeups.rs` did the same for a
  session's daemon.
- What changed (only `crates/cmux-usage-patterns`; no acpmux source line changed):
  - The gate follows the settings file by kernel file events (`cmux_config::watch::Watcher`, the
    same watch the daemon's settings owner uses: inotify on Linux, kqueue on macOS), not by a
    timer. The watch thread (`cmux-config-watch`) blocks in the kernel until the file, or a
    missing folder on the way to it, changes.
  - The flush thread `usage-patterns` starts the first time counting is on. It is parked while
    there is nothing to write. The first count after a write unparks it; it then writes at most
    every 30 s (`FLUSH_EVERY`). The first count after a quiet time is written at once.
  - So acpmux with the setting off has no thread of this crate that runs, and with the setting
    on it has none while nothing is counted.
- Cost: one more blocked thread and one more kernel watch (one inotify instance on Linux, one
  kqueue on macOS) in each process that counts. A watch that cannot start leaves the gate as it
  was read at the start of the process; there is no polling fallback.
- A behavior that is different: a `cmux usage-patterns reset` while nothing is counted is seen by
  a running recorder at its next write (right after its next count), not within 30 s. The reset
  still stays a reset: the file is not written back with the old counts.
- Not changed: what is counted, the file format, the setting, `_acpmux/stats`.
