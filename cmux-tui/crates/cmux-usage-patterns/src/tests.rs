use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("cmux-usage-patterns-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &Path, on: bool) -> PathBuf {
    let path = dir.join("cmux-next.json");
    std::fs::write(
        &path,
        format!("// a comment\n{{ \"diagnostics\": {{ \"usagePatterns\": {on} }} }}\n"),
    )
    .unwrap();
    path
}

#[test]
fn nothing_is_counted_while_the_setting_is_off() {
    let dir = scratch("off");
    let missing = dir.join("absent.json");
    let recorder = Recorder::open(&dir.join("out"), "session-a", missing);
    assert!(!recorder.enabled(), "a missing settings file is off");
    recorder.count("command", "split");
    recorder.peak("connections", 4);
    recorder.flush().unwrap();
    assert!(recorder.snapshot().is_empty());
    assert!(!recorder.path().exists(), "off writes no file");

    let off = config(&dir, false);
    let recorder = Recorder::open(&dir.join("out"), "session-a", off);
    recorder.count("command", "split");
    assert!(recorder.snapshot().is_empty());
}

#[test]
fn counts_flush_and_merge_across_writers() {
    let dir = scratch("merge");
    let on = config(&dir, true);
    let out = dir.join("out");
    let a = Recorder::open(&out, "session-a", on.clone());
    let b = Recorder::open(&out, "acpmux", on);
    assert!(a.enabled() && b.enabled());
    a.count("command", "split");
    a.count("command", "split");
    a.count("layout", "split-right");
    a.peak("connections.open", 3);
    b.count("command", "split");
    b.peak("connections.open", 5);
    a.flush().unwrap();
    b.flush().unwrap();

    let readout = read_dir(&out);
    assert_eq!(readout.writers.len(), 2);
    assert_eq!(readout.rollup.counts["command"]["split"], 3);
    assert_eq!(readout.rollup.counts["layout"]["split-right"], 1);
    assert_eq!(readout.rollup.peaks["connections.open"], 5);
    assert_eq!(readout.rollup.hours.iter().sum::<u64>(), 4);
    assert_eq!(readout.rollup.days.values().flat_map(|f| f.values()).sum::<u64>(), 4);

    // A reopened writer keeps its earlier counts.
    let again = Recorder::open(&out, "session-a", dir.join("cmux-next.json"));
    assert_eq!(again.snapshot().counts["command"]["split"], 2);
}

#[test]
fn keys_that_could_carry_content_count_as_other() {
    for bad in [
        "",
        "hello world",
        "https://example.com/?q=secret",
        "/Users/leo/private.txt",
        "~/notes",
        "me@example.com",
        "a=b",
        "C:\\Users\\leo",
        "../etc/passwd",
        "\"quoted\"",
        "file:/Users/leo/x",
        "Users/leo/notes",
        &"x".repeat(MAX_KEY_BYTES + 1),
    ] {
        assert_eq!(clean_key(bad), OTHER, "{bad:?} must not be kept");
    }
    for good in
        ["split", "tab.create_browser", "new-browser-tab", "claude", "session/new", "_acpmux/stats"]
    {
        assert_eq!(clean_key(good), good);
    }
}

#[test]
fn a_rollup_stays_bounded() {
    let mut rollup = Rollup::default();
    for i in 0..(MAX_KEYS_PER_FAMILY as u64 + 50) {
        rollup.add("command", &format!("k{i}"), 1, i);
    }
    assert_eq!(rollup.counts["command"].len(), MAX_KEYS_PER_FAMILY + 1);
    assert_eq!(rollup.counts["command"][OTHER], 50);
    for day in 0..(KEEP_DAYS as u64 + 30) {
        rollup.add("command", "k0", 1, day * 86_400_000);
    }
    assert_eq!(rollup.days.len(), KEEP_DAYS);
    assert_eq!(rollup.hours.len(), 24);
}

#[test]
fn a_reset_file_is_not_written_back() {
    let dir = scratch("reset");
    let on = config(&dir, true);
    let out = dir.join("out");
    let recorder = Recorder::open(&out, "session-a", on);
    recorder.count("command", "split");
    recorder.flush().unwrap();
    assert!(recorder.path().exists());
    recorder.count("command", "split");
    assert_eq!(reset_dir(&out).unwrap(), 1);
    recorder.flush().unwrap();
    assert!(!recorder.path().exists(), "the reset stays reset");
    assert!(recorder.snapshot().is_empty());
    recorder.count("command", "new-tab");
    recorder.flush().unwrap();
    assert_eq!(read_dir(&out).rollup.counts["command"].len(), 1);
}

#[test]
fn turning_the_setting_off_stops_counting_and_writes_what_was_counted() {
    let dir = scratch("toggle");
    let on = config(&dir, true);
    let recorder = Recorder::open(&dir.join("out"), "session-a", on.clone());
    recorder.count("command", "split");
    // The same size as the file that turned it on: the gate reads the file, it does not go by
    // the file's size or time.
    let off = "// a comment\n{ \"diagnostics\": { \"usagePatterns\": false} }\n";
    assert_eq!(off.len() as u64, std::fs::metadata(&on).unwrap().len());
    std::fs::write(&on, off).unwrap();
    assert!(!recorder.settings_changed());
    assert!(!recorder.enabled());
    recorder.count("command", "split");
    assert_eq!(recorder.snapshot().counts["command"]["split"], 1);
    assert!(recorder.path().exists(), "turning off writes what was counted");
}

/// The flush thread has nothing to do until something is counted: it stays parked (no timer),
/// writes the first count after a quiet time at once, and later counts at most every
/// [`FLUSH_EVERY`].
#[test]
fn nothing_is_due_to_be_written_until_something_is_counted() {
    let dir = scratch("due");
    let recorder = Recorder::open(&dir.join("out"), "session-a", config(&dir, true));
    assert_eq!(recorder.flush_wait(), None, "nothing counted: the flush thread stays parked");
    recorder.peak("connections.open", 0);
    assert_eq!(recorder.flush_wait(), None, "a peak that did not rise is nothing to write");
    recorder.count("command", "split");
    assert_eq!(recorder.flush_wait(), Some(Duration::ZERO), "the first count is written at once");
    recorder.flush().unwrap();
    assert_eq!(recorder.flush_wait(), None, "written: parked again");
    recorder.count("command", "split");
    let wait = recorder.flush_wait().expect("a count after a write is due");
    assert!(
        wait <= FLUSH_EVERY && wait > FLUSH_EVERY / 2,
        "the next write waits for the rest of FLUSH_EVERY, not {wait:?}"
    );

    let off = Recorder::open(&dir.join("out-off"), "session-a", config(&dir, false));
    off.count("command", "split");
    assert_eq!(off.flush_wait(), None, "off: nothing is counted, so nothing is ever due");
}

/// While counting was never on, a settings hint starts no thread; the first hint that finds it on
/// starts the flush thread once.
#[test]
fn the_flush_thread_starts_only_when_counting_is_on() {
    let dir = scratch("lazy");
    let recorder = Arc::new(Recorder::open(&dir.join("out"), "session-a", config(&dir, false)));
    recorder.follow_setting();
    recorder.follow_setting();
    assert!(recorder.flusher.get().is_none(), "off: no flush thread");
    config(&dir, true);
    recorder.follow_setting();
    let first = recorder.flusher.get().and_then(Option::as_ref).map(std::thread::Thread::id);
    assert!(first.is_some(), "on: the flush thread runs");
    config(&dir, false);
    recorder.follow_setting();
    config(&dir, true);
    recorder.follow_setting();
    let again = recorder.flusher.get().and_then(Option::as_ref).map(std::thread::Thread::id);
    assert_eq!(first, again, "one flush thread for the recorder's life");
}

/// The flush thread writes every count, not only the first: a count after a write wakes the
/// parked thread again, and the write comes when the time between two writes has passed.
#[test]
fn the_flush_thread_writes_each_count_that_comes_after_a_write() {
    let dir = scratch("later");
    let out = dir.join("out");
    let mut recorder = Recorder::open(&out, "session-a", config(&dir, true));
    recorder.flush_every = Duration::from_millis(50);
    let recorder = Arc::new(recorder);
    recorder.start_flusher();
    let splits =
        || read_dir(&out).rollup.counts.get("command").and_then(|keys| keys.get("split")).copied();
    for n in 1..=3 {
        recorder.count("command", "split");
        wait_for("the flush thread to write the count", || splits() == Some(n));
        wait_for("the flush thread to have nothing more to write", || {
            recorder.flush_wait().is_none()
        });
    }
}

/// A dropped recorder ends its flush thread at once, also while the thread waits for the time
/// of its next write (30 s away here).
#[test]
fn the_flush_thread_ends_with_its_recorder() {
    let dir = scratch("ends");
    let recorder = Arc::new(Recorder::open(&dir.join("out"), "session-a", config(&dir, true)));
    // The flush thread as `start_flusher` starts it, with a handle the test can wait on.
    let weak = Arc::downgrade(&recorder);
    let thread = std::thread::spawn(move || flush_loop(&weak));
    assert!(recorder.flusher.set(Some(thread.thread().clone())).is_ok());
    recorder.wake_flusher();
    recorder.count("command", "split");
    wait_for("the first count to be written", || recorder.path().exists());
    wait_for("the flush thread to have nothing more to write", || recorder.flush_wait().is_none());
    // Due in FLUSH_EVERY: the thread waits for that time now.
    recorder.count("command", "split");
    assert!(recorder.flush_wait().is_some_and(|wait| wait > FLUSH_EVERY / 2));
    drop(recorder);
    wait_for("the flush thread to end", || thread.is_finished());
    thread.join().unwrap();
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// End to end with the real watch: a change of the settings file turns counting on with no
/// poll, the first count is written by the flush thread, and turning it off writes the rest.
#[test]
fn a_watched_recorder_follows_the_settings_file() {
    let dir = scratch("watch");
    let settings = config(&dir, false);
    let out = dir.join("out");
    let recorder = Arc::new(Recorder::open(&out, "session-a", settings.clone()));
    let watch = recorder.watch().expect("the settings watch starts");
    assert!(!recorder.enabled());

    let splits =
        || read_dir(&out).rollup.counts.get("command").and_then(|keys| keys.get("split")).copied();
    config(&dir, true);
    wait_for("the watch to turn counting on", || recorder.enabled());
    recorder.count("command", "split");
    wait_for("the flush thread to write the first count", || splits() == Some(1));

    // Due only after the rest of FLUSH_EVERY, so the write below is the one that turning the
    // setting off makes.
    recorder.count("command", "split");
    std::fs::write(&settings, "{\"diagnostics\":{\"usagePatterns\":false}}").unwrap();
    wait_for("the watch to turn counting off", || !recorder.enabled());
    wait_for("turning off to write what was counted", || splits() == Some(2));
    drop(watch);
}

#[test]
fn the_dotted_key_also_turns_it_on() {
    let dir = scratch("dotted");
    let path = dir.join("cmux-next.json");
    std::fs::write(&path, "{\"diagnostics.usagePatterns\": true}").unwrap();
    assert!(setting_enabled(&path));
}

#[test]
fn opaque_ids_are_stable_and_do_not_show_the_id() {
    let a = opaque_id("machine-1234");
    assert_eq!(a, opaque_id("machine-1234"));
    assert_ne!(a, opaque_id("machine-1235"));
    assert_eq!(a.len(), 12);
    assert!(!a.contains("1234"));
    assert_eq!(clean_key(&a), a);
}

#[test]
fn utc_days_are_civil_dates() {
    assert_eq!(utc_day(0), "1970-01-01");
    assert_eq!(utc_day(951_782_400_000), "2000-02-29");
    assert_eq!(utc_day(1_793_836_800_000), "2026-11-05");
}

#[test]
fn pruning_keeps_the_newest_writers() {
    let dir = scratch("prune");
    for i in 0..5 {
        std::fs::write(dir.join(format!("w{i}.json")), "{}").unwrap();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(prune_writers(&dir, 3), 2);
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["w2.json", "w3.json", "w4.json"]);
}

#[test]
fn opaque_ids_are_sha_based_and_stable() {
    assert_eq!(opaque_id("main"), opaque_id("main"));
    assert_eq!(opaque_id("main").len(), 12);
}
