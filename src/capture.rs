use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::Sender;
use std::thread;

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
    use super::strip_non_sgr;

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
