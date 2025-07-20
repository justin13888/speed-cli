use std::{fs::File, io, path::Path};

/// Attempts to simply check if a file could be written or overwritten at the specified path.
pub fn can_write(file_path: &Path) -> io::Result<bool> {
    // Try to open the file in write mode, creating it if it doesn't exist
    match File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(file_path)
    {
        Ok(_) => Ok(true), // File can be written to
        Err(e) => Err(e),  // Errors (e.g., permission denied)
    }
}
