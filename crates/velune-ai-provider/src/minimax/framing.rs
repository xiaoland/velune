//! Content-free byte framing evidence. Raw line bytes never leave this object.
//! This observer does not normalize input or add delimiters for the SSE parser.
#[derive(Default, Clone, Copy)]
pub struct FramingEvidence {
    pub done_lines: u32,
    pub done_frames: u32,
    pub unfinished_data_frame: bool,
    pub unfinished_line: bool,
    pub unfinished_line_is_done: bool,
}
#[derive(Default)]
pub(super) struct FramingObserver {
    line: Vec<u8>,
    skip_lf: bool,
    data_pending: bool,
    done_pending: bool,
    done_lines: u32,
    done_frames: u32,
}
impl FramingObserver {
    pub fn push(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.skip_lf && *byte == b'\n' {
                self.skip_lf = false;
                continue;
            }
            self.skip_lf = false;
            if *byte == b'\r' || *byte == b'\n' {
                self.end_line();
                self.skip_lf = *byte == b'\r';
            } else {
                self.line.push(*byte);
            }
        }
    }
    fn data(line: &[u8]) -> Option<&[u8]> {
        let data = line.strip_prefix(b"data:")?;
        Some(data.strip_prefix(b" ").unwrap_or(data))
    }
    fn end_line(&mut self) {
        if self.line.is_empty() {
            if self.done_pending {
                self.done_frames += 1;
            }
            self.data_pending = false;
            self.done_pending = false;
        } else if let Some(data) = Self::data(&self.line) {
            self.data_pending = true;
            if data == b"[DONE]" {
                self.done_lines += 1;
                self.done_pending = true;
            }
        }
        self.line.clear();
    }
    pub fn evidence(&self) -> FramingEvidence {
        FramingEvidence {
            done_lines: self.done_lines,
            done_frames: self.done_frames,
            unfinished_data_frame: self.data_pending,
            unfinished_line: !self.line.is_empty(),
            unfinished_line_is_done: Self::data(&self.line) == Some(b"[DONE]".as_slice()),
        }
    }
}
