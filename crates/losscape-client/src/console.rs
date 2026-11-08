//! Console capture: what the process writes to stdout and stderr, as numbered lines
//! (docs/REQUIREMENTS.md, Design 3).
//!
//! The file descriptors themselves are redirected, so output of C extensions, CUDA and
//! subprocesses is included. A descriptor that was a terminal gets a pseudo-terminal, so
//! progress bars still see one and keep animating; anything else gets a pipe. One thread
//! per stream copies everything to the original destination and feeds a small screen
//! model that applies `\r`, `\n` and the ANSI sequences progress bars use (cursor up,
//! erase line). A tqdm bar becomes one line, re-sent at most once a second while it
//! changes, instead of thousands.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use losscape_core::proto::v0::{ConsoleLine, ConsoleStream};

/// Lines per stream that cursor-up can still reach and rewrite.
const WINDOW: usize = 200;
/// Longest line kept; the rest of a longer line is dropped.
pub const MAX_LINE_BYTES: usize = 16 * 1024;
/// A line already sent is sent again at most this often while it keeps changing.
const RESEND_EVERY: Duration = Duration::from_millis(250);
/// A sent line that just went empty is held back this long: progress bars are cleared and
/// redrawn around every message, and sending that moment would make them blink.
const CLEARED_HOLD_MS: i64 = 500;
/// Output stored per run; past it, output still reaches the terminal but isn't kept.
pub const MAX_CAPTURED_BYTES: u64 = 100 << 20;
/// Most lines per batch (the server takes up to 100k).
pub const MAX_LINES_PER_BATCH: usize = 10_000;
/// Longest escape sequence parsed; longer ones are dropped.
const MAX_ESCAPE: usize = 64;

struct Line {
    text: Vec<u8>,
    /// Assigned when first sent, so numbers follow the order lines appear.
    number: Option<u64>,
    dirty: bool,
    sent_at: Option<Instant>,
    /// When the line got its first byte (orders lines of the two streams).
    created_ms: i64,
    /// Changed since it was started (its first change sets `created_ms`).
    started: bool,
    /// Last change.
    time_ms: i64,
    /// Bytes of this line counted as stored (a resend replaces what was sent before).
    stored_len: u64,
}

impl Line {
    fn new(time_ms: i64) -> Self {
        Self {
            text: Vec::new(),
            number: None,
            dirty: false,
            sent_at: None,
            created_ms: time_ms,
            started: false,
            time_ms,
            stored_len: 0,
        }
    }

    fn touch(&mut self, time_ms: i64) {
        // Called after the change, so the text can't tell whether it's the first one.
        if !self.started {
            self.started = true;
            self.created_ms = time_ms;
        }
        self.dirty = true;
        self.time_ms = time_ms;
    }
}

enum Escape {
    None,
    Esc,
    /// Control sequence (`ESC [`), parameters so far.
    Csi(Vec<u8>),
    /// Operating system command (`ESC ]`, e.g. window title, hyperlink), until BEL or
    /// `ESC \`; true right after an ESC.
    Osc(bool),
}

/// The lines of one stream, as a terminal would show them.
struct Screen {
    stream: ConsoleStream,
    lines: VecDeque<Line>,
    cursor: usize,
    /// After `\r`: the next text replaces the line.
    carriage: bool,
    escape: Escape,
}

impl Screen {
    fn new(stream: ConsoleStream) -> Self {
        Self {
            stream,
            lines: VecDeque::from([Line::new(0)]),
            cursor: 0,
            carriage: false,
            escape: Escape::None,
        }
    }

    fn feed(&mut self, bytes: &[u8], time_ms: i64) {
        for &b in bytes {
            self.byte(b, time_ms);
        }
    }

    fn byte(&mut self, b: u8, time_ms: i64) {
        match std::mem::replace(&mut self.escape, Escape::None) {
            Escape::None => {}
            Escape::Esc => {
                match b {
                    b'[' => self.escape = Escape::Csi(Vec::new()),
                    b']' => self.escape = Escape::Osc(false),
                    _ => {} // other two-byte sequences: dropped
                }
                return;
            }
            Escape::Csi(mut params) => {
                if (0x40..=0x7e).contains(&b) {
                    self.csi(&params, b, time_ms);
                } else if params.len() < MAX_ESCAPE {
                    params.push(b);
                    self.escape = Escape::Csi(params);
                }
                return;
            }
            Escape::Osc(after_esc) => {
                if !(b == 0x07 || (after_esc && b == b'\\')) {
                    self.escape = Escape::Osc(b == 0x1b);
                }
                return;
            }
        }
        match b {
            b'\n' => self.newline(time_ms),
            b'\r' => self.carriage = true,
            0x1b => self.escape = Escape::Esc,
            0x08 => {
                // Backspace: drop the last character.
                let line = &mut self.lines[self.cursor];
                while let Some(last) = line.text.pop() {
                    if last & 0xc0 != 0x80 {
                        break; // removed a whole UTF-8 character
                    }
                }
                line.touch(time_ms);
            }
            b'\t' => self.put(&[b], time_ms),
            0x00..=0x1f | 0x7f => {}
            _ => self.put(&[b], time_ms),
        }
    }

    fn put(&mut self, bytes: &[u8], time_ms: i64) {
        let line = &mut self.lines[self.cursor];
        if std::mem::take(&mut self.carriage) {
            line.text.clear();
        }
        let room = MAX_LINE_BYTES.saturating_sub(line.text.len());
        line.text.extend_from_slice(&bytes[..bytes.len().min(room)]);
        line.touch(time_ms);
    }

    fn csi(&mut self, params: &[u8], final_byte: u8, time_ms: i64) {
        let first = std::str::from_utf8(params)
            .ok()
            .and_then(|p| p.split(';').next())
            .and_then(|n| n.parse::<usize>().ok());
        match final_byte {
            // Colors and styles stay in the text.
            b'm' => {
                let mut seq = Vec::with_capacity(params.len() + 3);
                seq.extend_from_slice(b"\x1b[");
                seq.extend_from_slice(params);
                seq.push(b'm');
                self.put(&seq, time_ms);
            }
            b'A' => self.cursor = self.cursor.saturating_sub(first.unwrap_or(1).max(1)),
            b'B' | b'E' => {
                for _ in 0..first.unwrap_or(1).max(1) {
                    self.down(time_ms);
                }
                self.carriage |= final_byte == b'E';
            }
            b'F' => {
                self.cursor = self.cursor.saturating_sub(first.unwrap_or(1).max(1));
                self.carriage = true;
            }
            // Cursor to column 1.
            b'G' if first.unwrap_or(1) <= 1 => self.carriage = true,
            b'K' => {
                // Erase in line. The column isn't tracked: the cursor is at the end of
                // the text, or at the start right after `\r`.
                let whole = first == Some(2);
                if whole || self.carriage {
                    let line = &mut self.lines[self.cursor];
                    if !line.text.is_empty() {
                        line.text.clear();
                        line.touch(time_ms);
                    }
                }
            }
            _ => {} // other movements, screen clears: ignored
        }
    }

    fn down(&mut self, time_ms: i64) {
        if self.cursor + 1 == self.lines.len() {
            self.lines.push_back(Line::new(time_ms));
        }
        self.cursor += 1;
    }

    fn newline(&mut self, time_ms: i64) {
        self.carriage = false;
        let line = &mut self.lines[self.cursor];
        if line.number.is_none() {
            line.touch(time_ms); // empty lines are kept too
        }
        self.down(time_ms);
        // Forget lines that are sent and out of reach.
        while self.lines.len() > WINDOW
            && self.cursor > 0
            && self.lines[0].number.is_some()
            && !self.lines[0].dirty
        {
            self.lines.pop_front();
            self.cursor -= 1;
        }
    }

    /// Lines ready to send: `(index, needs a number)`. A line is sent as soon as it has
    /// content, then again at most every `RESEND_EVERY` while it changes.
    fn ready(&self, now: Instant, force: bool) -> Vec<usize> {
        // A new line goes with what changed around it: a message pushing a bar down clears
        // the bar's old row, and alone the new row would show the bar twice.
        let new_line = self.lines.iter().any(|l| l.dirty && l.number.is_none());
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let being_redrawn = l.number.is_some()
                    && l.text.iter().all(u8::is_ascii_whitespace) // tqdm clears with spaces
                    && crate::run::now_ms() - l.time_ms < CLEARED_HOLD_MS;
                l.dirty
                    && (force
                        || (!being_redrawn
                            && (new_line
                                || l.sent_at
                                    .is_none_or(|at| now.duration_since(at) >= RESEND_EVERY))))
            })
            .map(|(i, _)| i)
            .collect()
    }
}

/// Both streams of a run's console, numbered together.
pub struct Console {
    screens: [Screen; 2],
    next_number: u64,
    stored_bytes: u64,
    full: bool,
}

impl Console {
    /// `first_line`: where numbering starts (after a resumed run's earlier output).
    pub fn new(first_line: u64) -> Self {
        Self {
            screens: [
                Screen::new(ConsoleStream::Stdout),
                Screen::new(ConsoleStream::Stderr),
            ],
            next_number: first_line,
            stored_bytes: 0,
            full: false,
        }
    }

    pub fn feed(&mut self, stream: ConsoleStream, bytes: &[u8], time_ms: i64) {
        if self.full {
            return;
        }
        let screen = &mut self.screens[(stream == ConsoleStream::Stderr) as usize];
        screen.feed(bytes, time_ms);
    }

    /// At most `max` lines to send now, oldest first; `force` (at finish) sends every
    /// change. Lines left over stay ready for the next call.
    pub fn take(&mut self, now: Instant, force: bool, max: usize) -> Vec<ConsoleLine> {
        let mut ready: Vec<(usize, usize)> = Vec::new();
        for (s, screen) in self.screens.iter().enumerate() {
            ready.extend(screen.ready(now, force).into_iter().map(|i| (s, i)));
        }
        // Lines already numbered first (by number), then new ones in the order they
        // appeared across both streams.
        ready.sort_by_key(|&(s, i)| {
            let line = &self.screens[s].lines[i];
            match line.number {
                Some(n) => (0, n as i64),
                None => (1, line.created_ms),
            }
        });
        ready.truncate(max);
        for &(s, i) in &ready {
            let line = &mut self.screens[s].lines[i];
            if line.number.is_none() {
                line.number = Some(self.next_number);
                self.next_number += 1;
            }
        }

        let mut out = Vec::with_capacity(ready.len());
        for (s, i) in ready {
            let stream = self.screens[s].stream;
            let line = &mut self.screens[s].lines[i];
            line.dirty = false;
            line.sent_at = Some(now);
            let len = line.text.len() as u64;
            self.stored_bytes = (self.stored_bytes + len).saturating_sub(line.stored_len);
            line.stored_len = len;
            out.push(ConsoleLine {
                line: line.number.expect("numbered above"),
                time_ms: line.time_ms,
                stream: stream as i32,
                text: String::from_utf8_lossy(&line.text).into_owned(),
            });
        }
        out.sort_by_key(|l| l.line);
        if self.stored_bytes > MAX_CAPTURED_BYTES && !self.full {
            self.full = true;
            out.push(ConsoleLine {
                line: self.next_number,
                time_ms: out.last().map_or(0, |l| l.time_ms),
                stream: ConsoleStream::Stderr as i32,
                text: format!(
                    "losscape: stopped storing console output after {} MB",
                    MAX_CAPTURED_BYTES >> 20
                ),
            });
            self.next_number += 1;
        }
        out
    }
}

#[cfg(unix)]
pub use capture::Capture;

#[cfg(unix)]
mod capture {
    use std::io;
    use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use losscape_core::proto::v0::ConsoleStream;
    use rustix::fs::{Mode, OFlags};
    use rustix::io::Errno;
    use rustix::pty::OpenptFlags;
    use rustix::termios::{OptionalActions, OutputModes};

    use super::Console;
    use crate::run::now_ms;

    /// stdout and stderr redirected into a [`Console`]. Restore them with [`Capture::stop`].
    pub struct Capture {
        streams: Vec<Redirect>,
    }

    struct Redirect {
        stream: ConsoleStream,
        /// The original destination (the copying thread writes to it too).
        saved: Arc<OwnedFd>,
        done: mpsc::Receiver<()>,
        thread: JoinHandle<()>,
    }

    fn standard(stream: ConsoleStream) -> BorrowedFd<'static> {
        if stream == ConsoleStream::Stderr {
            rustix::stdio::stderr()
        } else {
            rustix::stdio::stdout()
        }
    }

    fn redirect(stream: ConsoleStream, to: impl AsFd) -> io::Result<()> {
        if stream == ConsoleStream::Stderr {
            rustix::stdio::dup2_stderr(to)?;
        } else {
            rustix::stdio::dup2_stdout(to)?;
        }
        Ok(())
    }

    /// A pseudo-terminal the size of the terminal at `like`, without output processing
    /// (so `\n` stays `\n`). Returns (master, slave).
    fn pty(like: BorrowedFd<'_>) -> io::Result<(OwnedFd, OwnedFd)> {
        let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        rustix::pty::grantpt(&master)?;
        rustix::pty::unlockpt(&master)?;
        let name = rustix::pty::ptsname(&master, Vec::new())?;
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        if let Ok(size) = rustix::termios::tcgetwinsize(like) {
            rustix::termios::tcsetwinsize(&slave, size).ok();
        }
        if let Ok(mut term) = rustix::termios::tcgetattr(&slave) {
            term.output_modes.remove(OutputModes::OPOST);
            rustix::termios::tcsetattr(&slave, OptionalActions::Now, &term).ok();
        }
        Ok((master, slave))
    }

    fn write_all(fd: &OwnedFd, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match rustix::io::write(fd, bytes) {
                Ok(n) => bytes = &bytes[n..],
                Err(Errno::INTR) => {}
                Err(_) => return, // the original destination is gone: output is dropped
            }
        }
    }

    impl Capture {
        pub fn start(console: Arc<Mutex<Console>>) -> io::Result<Self> {
            let mut streams = Vec::new();
            for stream in [ConsoleStream::Stdout, ConsoleStream::Stderr] {
                match Redirect::start(stream, console.clone()) {
                    Ok(r) => streams.push(r),
                    Err(err) => {
                        Self { streams }.stop(Duration::ZERO);
                        return Err(err);
                    }
                }
            }
            Ok(Self { streams })
        }

        /// Puts stdout and stderr back and waits up to `timeout` for what is still in
        /// flight. Subprocesses that still hold the old descriptors keep being copied to
        /// the original destination.
        pub fn stop(self, timeout: Duration) {
            for r in &self.streams {
                redirect(r.stream, &*r.saved).ok();
            }
            let deadline = Instant::now() + timeout;
            for r in self.streams {
                let left = deadline.saturating_duration_since(Instant::now());
                if r.done.recv_timeout(left).is_ok() {
                    r.thread.join().ok();
                }
                // Otherwise the thread keeps copying until its writers are gone.
            }
        }
    }

    #[cfg(not(target_vendor = "apple"))]
    fn close_on_exec_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
        Ok(rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?)
    }

    /// Apple systems have no way to do this in one step, so a process started in between by
    /// another thread could inherit these. It would hold the pipe open, which at worst makes
    /// stopping the capture wait for its time limit.
    #[cfg(target_vendor = "apple")]
    fn close_on_exec_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
        use rustix::io::{FdFlags, fcntl_setfd};
        let (read, write) = rustix::pipe::pipe()?;
        fcntl_setfd(&read, FdFlags::CLOEXEC)?;
        fcntl_setfd(&write, FdFlags::CLOEXEC)?;
        Ok((read, write))
    }

    impl Redirect {
        fn start(stream: ConsoleStream, console: Arc<Mutex<Console>>) -> io::Result<Self> {
            let fd = standard(stream);
            let saved = Arc::new(rustix::io::fcntl_dupfd_cloexec(fd, 3)?);
            let (read, write) = if rustix::termios::isatty(fd) {
                pty(fd)?
            } else {
                close_on_exec_pipe()?
            };
            // The standard descriptor now points at the writer (not close-on-exec, so
            // subprocesses write there too); `write` itself closes when dropped.
            redirect(stream, &write)?;
            drop(write);

            let (tx, done) = mpsc::channel();
            let destination = saved.clone();
            let thread = std::thread::Builder::new()
                .name("losscape-console".into())
                .spawn(move || {
                    copy(&read, &destination, stream, &console);
                    tx.send(()).ok();
                })?;
            Ok(Self {
                stream,
                saved,
                done,
                thread,
            })
        }
    }

    /// Copies `read` to `saved` and into the console until every writer is closed
    /// (end of file on a pipe, EIO on a pseudo-terminal).
    fn copy(read: &OwnedFd, saved: &OwnedFd, stream: ConsoleStream, console: &Mutex<Console>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match rustix::io::read(read, &mut buf) {
                Ok(0) => return,
                Ok(n) => n,
                Err(Errno::INTR) => continue,
                Err(_) => return,
            };
            let bytes = &buf[..n];
            write_all(saved, bytes);
            // A bug in the console model must never stop the copy: the process would
            // block on a full pipe.
            if let Ok(mut console) = console.lock() {
                let fed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    console.feed(stream, bytes, now_ms())
                }));
                drop(fed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: ConsoleStream = ConsoleStream::Stdout;
    const ERR: ConsoleStream = ConsoleStream::Stderr;

    fn texts(lines: &[ConsoleLine]) -> Vec<(u64, &str)> {
        lines.iter().map(|l| (l.line, l.text.as_str())).collect()
    }

    #[test]
    fn splits_lines_and_keeps_empty_ones() {
        let mut c = Console::new(0);
        c.feed(OUT, b"hello\n\nwor", 1);
        c.feed(OUT, b"ld\n", 2);
        let lines = c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
        assert_eq!(texts(&lines), [(0, "hello"), (1, ""), (2, "world")]);
        assert!(
            c.take(Instant::now(), true, MAX_LINES_PER_BATCH).is_empty(),
            "nothing new"
        );
    }

    #[test]
    fn carriage_return_rewrites_the_line_and_limits_resends() {
        let mut c = Console::new(0);
        let t0 = Instant::now();
        c.feed(OUT, b"\r  0%|", 1);
        assert_eq!(
            texts(&c.take(t0, false, MAX_LINES_PER_BATCH)),
            [(0, "  0%|")],
            "first state at once"
        );
        for pct in 1..=50 {
            c.feed(OUT, format!("\r {pct:2}%|").as_bytes(), 1);
        }
        assert!(
            c.take(t0 + Duration::from_millis(100), false, MAX_LINES_PER_BATCH)
                .is_empty(),
            "throttled"
        );
        assert_eq!(
            texts(&c.take(t0 + RESEND_EVERY, false, MAX_LINES_PER_BATCH)),
            [(0, " 50%|")],
            "only the latest state, same line number"
        );
        c.feed(OUT, b"\r100%|\ndone\n", 1);
        assert_eq!(
            texts(&c.take(
                t0 + RESEND_EVERY + Duration::from_millis(50),
                false,
                MAX_LINES_PER_BATCH
            )),
            [(0, "100%|"), (1, "done")],
            "the new line goes now, with the rewrite before it"
        );
        assert!(
            c.take(t0 + RESEND_EVERY * 2, false, MAX_LINES_PER_BATCH)
                .is_empty()
        );
    }

    #[test]
    fn handles_cursor_up_and_erase_line_like_multi_bar_progress() {
        let mut c = Console::new(0);
        c.feed(ERR, b"bar A 10%\nbar B 10%\n", 1);
        c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
        // Redraw both: up two lines, erase, write.
        c.feed(ERR, b"\x1b[2A\r\x1b[2Kbar A 90%\n\x1b[2Kbar B 80%\n", 2);
        let lines = c.take(
            Instant::now() + Duration::from_secs(5),
            false,
            MAX_LINES_PER_BATCH,
        );
        assert_eq!(texts(&lines), [(0, "bar A 90%"), (1, "bar B 80%")]);
    }

    #[test]
    fn keeps_colors_and_drops_other_escapes() {
        let mut c = Console::new(0);
        c.feed(
            OUT,
            b"\x1b]0;title\x07\x1b[1;31merror\x1b[0m: x\x1b[?25l\n",
            1,
        );
        assert_eq!(
            texts(&c.take(Instant::now(), false, MAX_LINES_PER_BATCH)),
            [(0, "\x1b[1;31merror\x1b[0m: x")]
        );
    }

    #[test]
    fn erase_to_end_after_text_keeps_the_text() {
        // git and pip: "\rReceiving objects: 50%\x1b[K"
        let mut c = Console::new(0);
        c.feed(
            OUT,
            b"Receiving: 10% (long tail)\rReceiving: 50%\x1b[K\n",
            1,
        );
        assert_eq!(
            texts(&c.take(Instant::now(), false, MAX_LINES_PER_BATCH)),
            [(0, "Receiving: 50%")]
        );
    }

    #[test]
    fn numbers_both_streams_in_order_of_appearance() {
        let mut c = Console::new(7);
        c.feed(ERR, b"warning\n", 2);
        c.feed(OUT, b"first\n", 1);
        c.feed(OUT, b"third\n", 3);
        let lines = c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
        assert_eq!(texts(&lines), [(7, "first"), (8, "warning"), (9, "third")]);
        assert_eq!(lines[1].stream, ERR as i32);
    }

    #[test]
    fn a_bar_being_redrawn_is_not_sent_empty() {
        let mut c = Console::new(0);
        let t0 = Instant::now();
        c.feed(OUT, b"\r 10%|", 1);
        assert_eq!(
            texts(&c.take(t0, false, MAX_LINES_PER_BATCH)),
            [(0, " 10%|")]
        );
        let now = crate::run::now_ms();
        c.feed(OUT, b"\r\x1b[K", now);
        let later = t0 + RESEND_EVERY;
        assert!(
            c.take(later, false, MAX_LINES_PER_BATCH).is_empty(),
            "cleared a moment ago: the redraw is coming"
        );
        c.feed(OUT, b"\r 20%|", now);
        assert_eq!(
            texts(&c.take(later, false, MAX_LINES_PER_BATCH)),
            [(0, " 20%|")]
        );

        // Still empty a while later: it was really cleared.
        c.feed(OUT, b"\r\x1b[K", now - CLEARED_HOLD_MS);
        assert_eq!(
            texts(&c.take(later + RESEND_EVERY, false, MAX_LINES_PER_BATCH)),
            [(0, "")]
        );
    }

    #[test]
    fn rewrites_count_once_toward_the_limit() {
        let mut c = Console::new(0);
        let t0 = Instant::now();
        for i in 0..20u32 {
            c.feed(OUT, b"\r0123456789", 1);
            c.take(t0 + RESEND_EVERY * i, false, MAX_LINES_PER_BATCH);
        }
        assert_eq!(c.stored_bytes, 10, "each send replaces the one before");
    }

    #[test]
    fn a_line_sent_early_keeps_later_lines_in_order() {
        let mut c = Console::new(0);
        c.feed(OUT, b"top", 424);
        c.feed(ERR, b"started\n", 425);
        let first = c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
        assert_eq!(texts(&first), [(0, "top"), (1, "started")]);
        c.feed(OUT, b"\nbanner 1\nbanner 2\n", 427);
        c.feed(ERR, b"warning\n", 446);
        let lines = c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
        assert_eq!(
            texts(&lines),
            [(2, "banner 1"), (3, "banner 2"), (4, "warning")]
        );
    }

    #[test]
    fn caps_line_length_and_utf8_survives_split_writes() {
        let mut c = Console::new(0);
        let snowman = "☃".as_bytes();
        c.feed(OUT, &snowman[..1], 1);
        c.feed(OUT, &snowman[1..], 1);
        c.feed(OUT, &vec![b'x'; MAX_LINE_BYTES * 2], 1);
        c.feed(OUT, b"\n", 1);
        let lines = c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
        assert!(lines[0].text.starts_with('☃'));
        assert_eq!(lines[0].text.len(), MAX_LINE_BYTES);
    }

    #[test]
    fn backspace_removes_a_whole_character() {
        let mut c = Console::new(0);
        c.feed(OUT, "abé\x08\x08c\n".as_bytes(), 1);
        assert_eq!(
            texts(&c.take(Instant::now(), false, MAX_LINES_PER_BATCH)),
            [(0, "ac")]
        );
    }

    #[test]
    fn a_bar_cleared_with_spaces_is_not_sent_blank() {
        let mut c = Console::new(0);
        let start = Instant::now();
        let now = crate::run::now_ms();
        c.feed(ERR, b"bar 1%", now);
        c.take(start, false, MAX_LINES_PER_BATCH);
        c.feed(ERR, b"\r      \r", now);
        assert!(
            c.take(start + RESEND_EVERY, false, MAX_LINES_PER_BATCH)
                .is_empty()
        );
        c.feed(ERR, b"bar 2%", now);
        let sent = c.take(start + RESEND_EVERY, false, MAX_LINES_PER_BATCH);
        assert_eq!(texts(&sent), [(0, "bar 2%")]);
    }

    #[test]
    fn a_new_line_brings_recent_rewrites_with_it() {
        let mut c = Console::new(0);
        let start = Instant::now();
        let now = crate::run::now_ms();
        c.feed(ERR, b"bar 1%\r", now);
        assert_eq!(c.take(start, false, MAX_LINES_PER_BATCH).len(), 1);
        // Too soon to send the rewrite on its own.
        c.feed(ERR, b"bar 2%\r", now);
        assert!(c.take(start, false, MAX_LINES_PER_BATCH).is_empty());
        // A message takes the bar's row, and the bar moves to a new one: both go together.
        c.feed(ERR, b"warning\nbar 2%\r", now);
        let sent = c.take(start, false, MAX_LINES_PER_BATCH);
        let texts: Vec<_> = sent.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["warning", "bar 2%"]);
    }

    #[test]
    fn takes_at_most_max_lines_oldest_first() {
        let mut c = Console::new(0);
        for i in 0..25 {
            c.feed(OUT, format!("{i}\n").as_bytes(), i);
        }
        let now = Instant::now();
        let mut seen = Vec::new();
        loop {
            let batch = c.take(now, true, 10);
            if batch.is_empty() {
                break;
            }
            assert!(batch.len() <= 10);
            seen.extend(batch.into_iter().map(|l| (l.line, l.text)));
        }
        assert_eq!(seen.len(), 25);
        assert!(
            seen.iter().all(|(n, text)| text == &n.to_string()),
            "{seen:?}"
        );
    }

    #[test]
    fn forgets_old_lines_but_keeps_numbering() {
        let mut c = Console::new(0);
        for i in 0..1000 {
            c.feed(OUT, format!("line {i}\n").as_bytes(), i);
            if i % 10 == 0 {
                c.take(Instant::now(), false, MAX_LINES_PER_BATCH);
            }
        }
        let last = c.take(Instant::now(), true, MAX_LINES_PER_BATCH);
        assert_eq!(last.last().unwrap().line, 999);
        assert!(c.screens[0].lines.len() <= WINDOW + 11);
    }
}
