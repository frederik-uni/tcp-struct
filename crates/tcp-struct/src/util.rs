use crate::Error;

pub fn take_str(buffer: &mut &[u8]) -> crate::Result<String> {
    if buffer.len() < 4 {
        return Err(Error::BufferTooShort);
    }
    let size = {
        let size_bytes: [u8; 4] = buffer[..4].try_into().map_err(|_| Error::BufferTooShort)?;
        u32::from_ne_bytes(size_bytes)
    } as usize;

    if buffer.len() < 4 + size {
        return Err(Error::BufferTooShort);
    }

    let result = String::from_utf8(buffer[4..4 + size].to_vec()).map_err(Error::Utf8Error)?;

    *buffer = &buffer[4 + size..];

    Ok(result)
}

pub fn take_status_code(buffer: &mut &[u8]) -> crate::Result<u32> {
    if buffer.len() < 4 {
        return Err(Error::BufferTooShort);
    }
    let code = {
        let size_bytes: [u8; 4] = buffer[..4].try_into().map_err(|_| Error::BufferTooShort)?;
        u32::from_ne_bytes(size_bytes)
    };

    *buffer = &buffer[4..];

    Ok(code)
}
