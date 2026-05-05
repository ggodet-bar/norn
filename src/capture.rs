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
            if tx.send(LogLine { raw }).is_err() {
                break;
            }
        }
    });
}
