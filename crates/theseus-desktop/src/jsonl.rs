//! Strict JSONL framing for `pi --mode rpc`.
//!
//! Split records on `\n` only. Strip a trailing `\r` (so `\r\n` works).
//! Do **not** treat U+2028 / U+2029 as record boundaries — they are valid
//! inside JSON strings.

use std::io::Read;
use std::sync::mpsc::Sender;

/// Drain complete JSONL records from `buf`. Leaves a partial trailing record.
pub fn drain_jsonl_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        let mut line: Vec<u8> = buf.drain(..=pos).collect();
        line.pop(); // `\n`
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }
        match String::from_utf8(line) {
            Ok(s) if !s.is_empty() => lines.push(s),
            _ => {}
        }
    }
    lines
}

/// Read stdout until EOF, sending one JSONL record per channel message.
pub fn read_jsonl<R: Read>(mut reader: R, tx: Sender<String>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match reader.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                for line in drain_jsonl_lines(&mut buf) {
                    if tx.send(line).is_err() {
                        return;
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !buf.is_empty() {
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        if let Ok(s) = String::from_utf8(buf) {
            if !s.is_empty() {
                let _ = tx.send(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;

    #[test]
    fn splits_on_lf_and_strips_cr() {
        let mut buf = b"{\"a\":1}\r\n{\"b\":2}\n".to_vec();
        let lines = drain_jsonl_lines(&mut buf);
        assert_eq!(lines, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
        assert!(buf.is_empty());
    }

    #[test]
    fn keeps_partial_record() {
        let mut buf = b"{\"a\":".to_vec();
        let lines = drain_jsonl_lines(&mut buf);
        assert!(lines.is_empty());
        assert_eq!(buf, b"{\"a\":");
    }

    #[test]
    fn does_not_split_on_unicode_separators() {
        let sep = "\u{2028}";
        let rec = format!(r#"{{"type":"x","s":"a{sep}b"}}"#);
        let mut buf = format!("{rec}\n").into_bytes();
        let lines = drain_jsonl_lines(&mut buf);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{2028}'));
        assert_eq!(lines[0].matches('\n').count(), 0);
    }

    #[test]
    fn reader_forwards_records() {
        let (tx, rx) = mpsc::channel();
        let data = b"{\"type\":\"a\"}\r\n{\"type\":\"b\"}\n";
        read_jsonl(Cursor::new(&data[..]), tx);
        let got: Vec<String> = rx.iter().collect();
        assert_eq!(got, vec![r#"{"type":"a"}"#, r#"{"type":"b"}"#]);
    }
}
