use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

pub const MAX_FRAME_SIZE: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame exceeds maximum size")]
    FrameTooLarge,
    #[error("frame is not valid UTF-8")]
    InvalidUtf8,
    #[error("frame is not valid JSON")]
    InvalidJson,
    #[error("frame I/O failed")]
    Io(#[from] io::Error),
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, FrameError> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_SIZE {
        return Err(FrameError::FrameTooLarge);
    }

    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    let json = std::str::from_utf8(&payload).map_err(|_| FrameError::InvalidUtf8)?;
    serde_json::from_str(json).map_err(|_| FrameError::InvalidJson)
}

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(value).map_err(|_| FrameError::InvalidJson)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(FrameError::FrameTooLarge);
    }

    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(&payload)?;
    Ok(())
}
