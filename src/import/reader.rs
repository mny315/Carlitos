//! A document can occupy only a slice of its provider's file descriptor.
//! Parsers see offsets relative to that slice and cannot read its neighbours.
use std::io::{self, Read, Seek, SeekFrom};

pub(crate) struct DocumentReader<R> {
    inner: R,
    offset: u64,
    length: u64,
    position: u64,
}
impl<R: Read + Seek> DocumentReader<R> {
    pub fn new(mut inner: R, offset: u64, length: Option<u64>) -> io::Result<Self> {
        let end = inner.seek(SeekFrom::End(0))?;
        let available = end.checked_sub(offset).ok_or_else(invalid_range)?;
        let length = length.unwrap_or(available);
        if length > available {
            return Err(invalid_range());
        }
        inner.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            inner,
            offset,
            length,
            position: 0,
        })
    }
    pub fn len(&self) -> u64 {
        self.length
    }
}
fn invalid_range() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Invalid document offset or length",
    )
}
impl<R: Read> Read for DocumentReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = (self.length - self.position).min(buffer.len() as u64) as usize;
        let read = self.inner.read(&mut buffer[..count])?;
        self.position += read as u64;
        Ok(read)
    }
}
impl<R: Seek> Seek for DocumentReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        if position < 0 || position > i128::from(self.length) {
            return Err(invalid_range());
        }
        self.inner
            .seek(SeekFrom::Start(self.offset + position as u64))?;
        self.position = position as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn document_offsets_and_length_bound_every_read_and_seek() -> io::Result<()> {
        let mut reader = DocumentReader::new(Cursor::new(b"prefixAUDIOsuffix"), 6, Some(5))?;
        assert_eq!(reader.len(), 5);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"AUDIO");
        assert_eq!(reader.seek(SeekFrom::End(-2))?, 3);
        let mut tail = [0; 8];
        assert_eq!(reader.read(&mut tail)?, 2);
        assert_eq!(&tail[..2], b"IO");
        assert!(reader.seek(SeekFrom::Current(1)).is_err());
        assert!(reader.seek(SeekFrom::Start(u64::MAX)).is_err());
        assert!(reader.seek(SeekFrom::End(-6)).is_err());
        assert_eq!(reader.stream_position()?, 5);
        assert!(DocumentReader::new(Cursor::new(b"short"), 6, None).is_err());
        assert!(DocumentReader::new(Cursor::new(b"short"), 1, Some(5)).is_err());
        assert_eq!(
            DocumentReader::new(Cursor::new(b"short"), 1, None)?.len(),
            4
        );
        Ok(())
    }
}
