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
    timer. The watch thread (`cmux-config-watch`) is blocked in the kernel. It wakes when an
    entry of the settings folder changes, and it rereads the setting only when that entry is the
    settings file. While the settings folder is missing, it wakes for an entry that is made,
    removed or renamed in the nearest folder that exists, and rereads nothing until the settings
    folder appears (the settings owner's watch does the same).
  - The flush thread `usage-patterns` starts the first time counting is on. It is parked while
    there is nothing to write. The first count after a write unparks it; it then writes at most
    every 30 s (`FLUSH_EVERY`). The first count after a quiet time is written at once.
  - So an idle acpmux does not wake for this crate: not with the setting off, and not with the
    setting on while nothing is counted.
- Cost: one more blocked thread and one more kernel watch (one inotify instance on Linux, one
  kqueue on macOS) in each session daemon and in acpmux, also while the setting is off. The
  flush thread is a second thread, only after counting was on. `CMUX_USAGE_PATTERNS` set: no
  watch (no save can change the gate).
- When the watch cannot start (no free inotify instance): this process does not count at all.
  Without the watch it could not see the person turn the setting off. There is no polling
  fallback, and nothing reports the failure (the settings owner's watch is silent the same way).
- A behavior that is different: a `cmux usage-patterns reset` is seen by a running recorder at
  its next write, as before. Before, a write pass ran every 30 s, so the recorder dropped what it
  held within 30 s. Now there is no write pass while nothing is counted: the recorder drops what
  it held at the write that its next count starts, and that count (with any other made before
  that write) goes with it. The reset stays a reset: the old counts are never written back.
- A second behavior that is different, on Linux only: a settings file that becomes a symlink
  after the process started is not followed to its new target (the 5 s check read through the
  link; the settings owner's watch has the same limit). On macOS a save of the target is seen
  through the watch on the file itself.
- Not changed: what is counted, the file format, the setting, `_acpmux/stats`.
- Found by the review in the code as #40554 merged it, not changed here: `flush` holds the state
  lock across one `stat` and the serialization, so a count can wait behind them; `written` is set
  before a write that can fail, and the next flush then reads the missing file as a reset; the
  managed (MDM) value of `diagnostics.usagePatterns` is not read; no host calls
  `cmux_usage_patterns::flush()` at shutdown, so the counts of the last 30 s of a process can be
  lost.
