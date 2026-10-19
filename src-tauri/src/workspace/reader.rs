use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::explorer::validate_read_path;
use super::models::FileChunk;
use super::settings::load_settings;

pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;
pub const MAX_CHUNK_SIZE: usize = 64 * 1024;

/// Reads a bounded chunk of an ordinary local file in UTF-8 text format.
/// Enforces all path, system, secret, and symlink safety restrictions.
/// Handles UTF-8 boundaries, binary rejection, offset bounds, and overflow cleanly.
/// Never persists or logs file contents; error messages are content-free.
pub fn read_local_file_chunk_impl(
    config_dir: &Path,
    path_str: &str,
    offset: u64,
    chunk_size: Option<u32>,
) -> Result<FileChunk, String> {
    // 1. Validate chunk size parameter
    let bounded_chunk = match chunk_size {
        Some(0) => return Err("Invalid chunk size: must be greater than zero".to_string()),
        Some(sz) => (sz as usize).min(MAX_CHUNK_SIZE),
        None => DEFAULT_CHUNK_SIZE,
    };

    // 2. Validate path safety, system/secret exclusions, and regular file type
    let canonical_path = validate_read_path(config_dir, path_str)?;
    let canonical_path_str = canonical_path.to_string_lossy().to_string();

    // 3. Inspect metadata and check offset bounds
    let metadata = std::fs::metadata(&canonical_path)
        .map_err(|_| "Cannot inspect file metadata".to_string())?;
    let total_bytes = metadata.len();

    if offset > total_bytes {
        return Err("Offset exceeds file size".to_string());
    }

    // EOF reached at exact boundary
    if offset == total_bytes {
        return Ok(FileChunk {
            path: canonical_path_str,
            text: String::new(),
            offset,
            next_offset: offset,
            total_bytes,
            eof: true,
        });
    }

    // Calculate bytes to read with overflow protection
    let remaining = total_bytes
        .checked_sub(offset)
        .ok_or_else(|| "Offset calculation overflow".to_string())?;
    let bytes_to_read = (remaining.min(bounded_chunk as u64)) as usize;

    // Open file and seek
    let mut file = File::open(&canonical_path).map_err(|_| "Failed to open file".to_string())?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "Failed to seek file".to_string())?;

    let mut buffer = vec![0u8; bytes_to_read];
    file.read_exact(&mut buffer)
        .map_err(|_| "Failed to read file chunk".to_string())?;

    // Reject binary content containing null bytes
    if buffer.contains(&0) {
        return Err("File contains binary or unsupported text encoding".to_string());
    }

    let reaches_eof = (offset + bytes_to_read as u64) >= total_bytes;

    // Validate UTF-8 and handle character boundaries
    match std::str::from_utf8(&buffer) {
        Ok(valid_str) => Ok(FileChunk {
            path: canonical_path_str,
            text: valid_str.to_string(),
            offset,
            next_offset: offset + bytes_to_read as u64,
            total_bytes,
            eof: reaches_eof,
        }),
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            let error_len = e.error_len();

            if error_len.is_some() {
                // Invalid byte sequence in stream
                return Err("File contains binary or unsupported text encoding".to_string());
            }

            // Incomplete code point at the end of the buffer
            if reaches_eof {
                // Unexpected EOF inside a multi-byte character
                return Err("File contains binary or unsupported text encoding".to_string());
            }

            if valid_up_to > 0 {
                let valid_str = std::str::from_utf8(&buffer[..valid_up_to])
                    .map_err(|_| "Failed to decode UTF-8 slice".to_string())?;
                Ok(FileChunk {
                    path: canonical_path_str,
                    text: valid_str.to_string(),
                    offset,
                    next_offset: offset + valid_up_to as u64,
                    total_bytes,
                    eof: false,
                })
            } else {
                // valid_up_to == 0: chunk size was smaller than a single multi-byte character
                let lead = buffer[0];
                let char_len = if (lead & 0b1110_0000) == 0b1100_0000 {
                    2
                } else if (lead & 0b1111_0000) == 0b1110_0000 {
                    3
                } else if (lead & 0b1111_1000) == 0b1111_0000 {
                    4
                } else {
                    return Err("File contains binary or unsupported text encoding".to_string());
                };

                if offset + char_len as u64 > total_bytes {
                    return Err("File contains binary or unsupported text encoding".to_string());
                }

                let mut char_buf = vec![0u8; char_len];
                file.seek(SeekFrom::Start(offset))
                    .map_err(|_| "Failed to seek file".to_string())?;
                file.read_exact(&mut char_buf)
                    .map_err(|_| "Failed to read character bytes".to_string())?;

                if char_buf.contains(&0) {
                    return Err("File contains binary or unsupported text encoding".to_string());
                }

                let s = std::str::from_utf8(&char_buf)
                    .map_err(|_| "File contains binary or unsupported text encoding".to_string())?;
                let next_off = offset + char_len as u64;
                Ok(FileChunk {
                    path: canonical_path_str,
                    text: s.to_string(),
                    offset,
                    next_offset: next_off,
                    total_bytes,
                    eof: next_off >= total_bytes,
                })
            }
        }
    }
}

/// Authorizes a local file for provider sharing in a future tool loop.
/// Rechecks all read safety restrictions PLUS ensures the file resides inside an
/// approved project root that has an active provider-sharing grant.
/// Also ensures that symlinks/junctions do not escape the containing project root.
/// Returns the canonical path of the authorized file.
/// Does NOT read file contents or send them to any provider.
pub fn authorize_provider_file(config_dir: &Path, file_path: &Path) -> Result<PathBuf, String> {
    let path_str = file_path
        .to_str()
        .ok_or_else(|| "Invalid file path encoding".to_string())?;
    let canonical = validate_read_path(config_dir, path_str)?;

    let settings = load_settings(config_dir)?;

    // Find the approved project root containing this file
    let mut containing_root = None;
    for root in &settings.project_roots {
        if let Ok(canon_root) = dunce::canonicalize(root) {
            if canonical.starts_with(&canon_root) {
                containing_root = Some((root.clone(), canon_root));
                break;
            }
        }
    }

    let (root_str, canon_root) = match containing_root {
        Some(pair) => pair,
        None => return Err("File is outside approved project folders".to_string()),
    };

    // Recheck current provider sharing grant
    let is_granted = settings.provider_sharing_grants.iter().any(|g| {
        g == &root_str
            || dunce::canonicalize(g)
                .map(|cg| cg == canon_root)
                .unwrap_or(false)
    });

    if !is_granted {
        return Err("Provider sharing is not enabled for this project folder".to_string());
    }

    // Verify input path also stays within the containing project root
    if !file_path.starts_with(&canon_root) && !file_path.starts_with(&root_str) {
        return Err("File path escapes approved project folder".to_string());
    }

    // Verify no intermediate symlink escapes the project root
    let mut current = PathBuf::new();
    for comp in file_path.components() {
        current.push(comp);
        if let Ok(meta) = std::fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = dunce::canonicalize(&current) {
                    if !target.starts_with(&canon_root) {
                        return Err("Symlink or junction escapes project root".to_string());
                    }
                }
            }
        }
    }

    Ok(canonical)
}
