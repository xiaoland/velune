//! Borrowed Pi wire envelopes. Count escaped JSON/base64 before allocating,
//! then serialize into a fixed slice: no Vec growth or full-image scratch.
use super::*;
use serde::ser::{SerializeMap, SerializeSeq};
use std::{
    fmt,
    io::{self, Write},
};

pub(super) fn encode(operation: &PiOperation, id: &str, available: usize) -> Result<Vec<u8>> {
    let mut count = Count {
        used: 1,
        limit: available,
    }; // JSONL newline
    if available == 0 {
        return Err(full());
    }
    serde_json::to_writer(
        &mut count,
        &Wire {
            operation,
            id,
            preflight: true,
        },
    )?;
    let length = count.used;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| full())?;
    // Charge the allocation actually obtained, including allocator overcapacity.
    if bytes.capacity() > available {
        return Err(full());
    }
    bytes.resize(length, 0);
    #[cfg(test)]
    if matches!(operation, PiOperation::Prompt(_) | PiOperation::Steer(_)) {
        INPUT_ENCODINGS.with(|count| count.set(count.get() + 1));
    }
    let mut output = io::Cursor::new(&mut bytes[..length - 1]);
    serde_json::to_writer(
        &mut output,
        &Wire {
            operation,
            id,
            preflight: false,
        },
    )?;
    if output.position() != (length - 1) as u64 {
        return Err(full());
    }
    bytes[length - 1] = b'\n';
    Ok(bytes)
}
fn full() -> Error {
    Error::Transport("Pi encoded payload capacity full".into())
}
struct Count {
    used: usize,
    limit: usize,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .used
            .checked_add(bytes.len())
            .filter(|next| *next <= self.limit)
            .ok_or_else(|| io::Error::other("Pi encoded payload capacity full"))?;
        self.used = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Wire<'a> {
    operation: &'a PiOperation,
    id: &'a str,
    preflight: bool,
}
impl Serialize for Wire<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use PiOperation::*;
        if let DialogReply(value) = self.operation {
            return value.serialize(serializer);
        }
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry(
            "type",
            self.operation
                .command_type()
                .map_err(serde::ser::Error::custom)?,
        )?;
        map.serialize_entry("id", self.id)?;
        match self.operation {
            Prompt(input) | Steer(input) => {
                map.serialize_entry("message", &input.text)?;
                map.serialize_entry(
                    "images",
                    &Images {
                        images: &input.images,
                        preflight: self.preflight,
                    },
                )?;
            }
            ThinkingLevel(level) => map.serialize_entry("level", level)?,
            SetModel { provider, model_id } => {
                map.serialize_entry("provider", provider)?;
                map.serialize_entry("modelId", model_id)?;
            }
            ProjectionSync => {
                map.serialize_entry("message", &format!("/velune-projection-sync {}", self.id))?
            }
            SyncSelection => map.serialize_entry("message", "/velune-sync-selection")?,
            Background(value) => map.serialize_entry("message", &BackgroundMessage(value))?,
            Rewind(id) => map.serialize_entry("message", &RewindMessage(id))?,
            Fork(id) => map.serialize_entry("entryId", id)?,
            SetName(name) => map.serialize_entry("name", name)?,
            SwitchSession(path) => map.serialize_entry("sessionPath", path)?,
            _ => {}
        }
        map.end()
    }
}
struct Images<'a> {
    images: &'a [crate::TurnImage],
    preflight: bool,
}
impl Serialize for Images<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.images.len()))?;
        for image in self.images {
            sequence.serialize_element(&Image {
                image,
                preflight: self.preflight,
            })?;
        }
        sequence.end()
    }
}
struct Image<'a> {
    image: &'a crate::TurnImage,
    preflight: bool,
}
impl Serialize for Image<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("type", "image")?;
        map.serialize_entry(
            "data",
            &ImageData {
                bytes: self.image.bytes(),
                preflight: self.preflight,
            },
        )?;
        map.serialize_entry("mimeType", self.image.kind.media_type())?;
        map.end()
    }
}
struct ImageData<'a> {
    bytes: &'a [u8],
    preflight: bool,
}
impl Serialize for ImageData<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if self.preflight {
            let length = base64::encoded_len(self.bytes.len(), true)
                .ok_or_else(|| serde::ser::Error::custom("Pi base64 length overflow"))?;
            // Base64 has no JSON escapes. Counting equivalent ASCII avoids
            // encoding any image until the complete aggregate frame fits.
            serializer.collect_str(&AsciiLength(length))
        } else {
            serializer.collect_str(&base64::display::Base64Display::new(
                self.bytes,
                &base64::engine::general_purpose::STANDARD,
            ))
        }
    }
}
struct AsciiLength(usize);
impl fmt::Display for AsciiLength {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const CHUNK: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let mut remaining = self.0;
        while remaining != 0 {
            let count = remaining.min(CHUNK.len());
            formatter.write_str(&CHUNK[..count])?;
            remaining -= count;
        }
        Ok(())
    }
}
struct RewindMessage<'a>(&'a str);
impl Serialize for RewindMessage<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl fmt::Display for RewindMessage<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("/velune-rewind ")?;
        formatter.write_str(self.0)
    }
}
struct BackgroundMessage<'a>(&'a Value);
impl Serialize for BackgroundMessage<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl fmt::Display for BackgroundMessage<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("/velune-background-work ")?;
        let mut encoder =
            base64::write::EncoderWriter::new(FormatWriter(formatter), &URL_SAFE_NO_PAD);
        serde_json::to_writer(&mut encoder, self.0).map_err(|_| fmt::Error)?;
        encoder.finish().map_err(|_| fmt::Error)?;
        Ok(())
    }
}
struct FormatWriter<'a, 'b>(&'a mut fmt::Formatter<'b>);
impl Write for FormatWriter<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
        self.0
            .write_str(text)
            .map_err(|_| io::Error::other("Pi base64 output capacity full"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
