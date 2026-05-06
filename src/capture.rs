use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub struct LogLine {
    pub raw: String,
}

/// Spawn a thread that reads `reader` line-by-line and forwards each line on
/// `tx`. The thread exits on EOF or send failure.
pub fn pipe_into<R>(reader: R, tx: Sender<LogLine>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            let Ok(raw) = line else { break };
            let raw = strip_non_sgr(&raw);
            if tx.send(LogLine { raw }).is_err() {
                break;
            }
        }
    });
}

/// Poll interval between EOF retries when tailing a file. 100ms keeps the UI
/// snappy without measurable idle cost (one cheap syscall per tick).
const TAIL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Spawn a thread that follows `reader` `tail -f`-style: read appended bytes
/// as they arrive, and on EOF sleep `TAIL_POLL_INTERVAL` and try again. Only
/// complete lines (terminated by `\n`) are forwarded — a partial trailing
/// line stays buffered until its newline arrives, so a producer that flushes
/// mid-line doesn't surface as two split rows. The thread exits on send
/// failure or hard I/O error; EOF alone is not terminal.
pub fn tail_into<R>(reader: R, tx: Sender<LogLine>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = String::new();
        loop {
            match reader.read_line(&mut buf) {
                Ok(0) => thread::sleep(TAIL_POLL_INTERVAL),
                Ok(_) => {
                    if !buf.ends_with('\n') {
                        // Partial line: producer hasn't flushed the newline
                        // yet. Keep accumulating across iterations.
                        continue;
                    }
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                    let raw = strip_non_sgr(&buf);
                    if tx.send(LogLine { raw }).is_err() {
                        break;
                    }
                    buf.clear();
                }
                Err(_) => break,
            }
        }
    });
}

/// Drop ANSI control sequences that move the cursor or rewrite the screen
/// while preserving SGR (`ESC [ ... m`) styling. Producers that mix progress
/// bars or full-screen redraws with normal log output otherwise leave
/// fragments in the buffer that read as ghost characters once `ansi-to-tui`
/// turns the stream into cells.
fn strip_non_sgr(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'[' => {
                    // CSI: skip params/intermediates until a final byte
                    // (0x40-0x7E). Keep only SGR (`m`); drop cursor moves,
                    // erases, mode toggles, etc.
                    let start = i;
                    let mut j = i + 2;
                    while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                        j += 1;
                    }
                    if j < bytes.len() {
                        if bytes[j] == b'm' {
                            out.extend_from_slice(&bytes[start..=j]);
                        }
                        i = j + 1;
                    } else {
                        i = bytes.len();
                    }
                    continue;
                }
                b']' => {
                    // OSC: terminated by BEL (0x07) or ST (`ESC \`).
                    let mut j = i + 2;
                    while j < bytes.len() {
                        if bytes[j] == 0x07 {
                            j += 1;
                            break;
                        }
                        if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                            j += 2;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
                c => {
                    // Bare ESC sequence: optional intermediate byte
                    // (0x20-0x2F, e.g. charset designations like `ESC ( B`)
                    // followed by a final. Drop the whole thing.
                    let extra = if (0x20..=0x2f).contains(&c) { 1 } else { 0 };
                    i += 2 + extra;
                    continue;
                }
            }
        }
        // Bare carriage returns and backspaces would shove following text
        // back over what was just rendered.
        if b == b'\r' || b == 0x08 {
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    // Filter only removes whole ASCII control sequences, so any multibyte
    // UTF-8 in the source survives intact.
    String::from_utf8(out).expect("filter preserves UTF-8 boundaries")
}

#[cfg(test)]
mod tests {
    use super::{LogLine, strip_non_sgr, tail_into};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn tail_into_emits_appended_and_buffers_partial_lines() {
        let path = std::env::temp_dir().join(format!(
            "norn_tail_{}_{:?}.log",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let mut f = File::create(&path).unwrap();
            writeln!(f, "first").unwrap();
        }

        let (tx, rx) = mpsc::channel::<LogLine>();
        tail_into(File::open(&path).unwrap(), tx);

        let line = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(line.raw, "first");

        let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(writer, "second").unwrap();
        writeln!(writer, "third").unwrap();
        let l2 = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let l3 = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(l2.raw, "second");
        assert_eq!(l3.raw, "third");

        // Partial line should be held back until its newline arrives.
        write!(writer, "partial").unwrap();
        writer.flush().unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "partial line emitted prematurely"
        );
        writeln!(writer, "-rest").unwrap();
        let l4 = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(l4.raw, "partial-rest");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keeps_sgr() {
        let s = "\x1b[31mred\x1b[0m";
        assert_eq!(strip_non_sgr(s), s);
    }

    #[test]
    fn drops_cursor_and_erase() {
        let s = "before\x1b[2J\x1b[Hmid\x1b[Kend";
        assert_eq!(strip_non_sgr(s), "beforemidend");
    }

    #[test]
    fn drops_osc_and_bare_esc() {
        let s = "a\x1b]0;title\x07b\x1b(Bc";
        assert_eq!(strip_non_sgr(s), "abc");
    }

    #[test]
    fn drops_carriage_return_and_backspace() {
        assert_eq!(strip_non_sgr("ab\rcd\x08e"), "abcde");
    }

    #[test]
    fn preserves_utf8() {
        let s = "héllo \x1b[1mwörld\x1b[0m";
        assert_eq!(strip_non_sgr(s), s);
    }
}
