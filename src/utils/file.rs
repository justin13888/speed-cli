use std::{fs::File, io, io::Write as _, path::Path};

/// Attempts to simply check if a file could be written or overwritten at the specified path.
pub fn can_write(file_path: &Path) -> io::Result<bool> {
    // Try to open the file in write mode
    match File::options().write(true).open(file_path) {
        Ok(_) => Ok(true), // File can be written to
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false), // File does not exist
        Err(e) => Err(e),  // Other errors (e.g., permission denied)
    }
}

// /// Attempts to write content to a file at the specified path.
// /// If the file does not exist, it will be created.
// /// If the file exists, it will be truncated to zero length before writing.
// pub fn attempt_write(file_path: &Path, content: &[u8]) -> io::Result<()> {
//     // OpenOptions gives fine-grained control over how the file is opened
//     // Here, we create a new file, or truncate it if it exists.
//     // Consider `create_new(true)` if you strictly want to avoid overwriting.
//     let mut file = File::options()
//         .write(true)
//         .create(true) // Create the file if it doesn't exist
//         .truncate(true) // Truncate to 0 length if it exists
//         .open(file_path)?;

//     file.write_all(content)?;
//     Ok(())
// }
