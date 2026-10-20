use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::discovery::{is_cache_or_generated_dir, is_symlink_or_reparse};
use super::explorer::{
    get_shareable_project_roots_impl, is_blocked_name, is_dos_device_name, is_drive_root,
    is_secret_directory, is_system_directory, is_unc_or_device_path, is_user_profile_root,
    is_yuki_internal_path,
};

pub const DEFAULT_MAX_DEPTH: usize = 4;
pub const MAX_ALLOWED_DEPTH: usize = 8;
pub const DEFAULT_MAX_FILE_RESULTS: usize = 20;
pub const MAX_ALLOWED_FILE_RESULTS: usize = 50;
pub const DEFAULT_MAX_TEXT_RESULTS: usize = 20;
pub const MAX_ALLOWED_TEXT_RESULTS: usize = 50;
pub const MAX_DIRECTORIES_SCANNED: usize = 1000;
pub const MAX_ENTRIES_INSPECTED: usize = 10_000;
pub const MAX_SEARCH_DURATION: Duration = Duration::from_secs(5);
pub const MAX_BYTES_INSPECTED_PER_SEARCH: u64 = 4 * 1024 * 1024; // 4 MB total file inspection limit
pub const SEARCH_READ_CHUNK_SIZE: usize = 32 * 1024; // 32 KiB streaming chunks
pub const MAX_SNIPPET_LINE_LENGTH: usize = 200;
pub const MAX_SNIPPET_CONTEXT_CHARS: usize = 60;
pub const MAX_QUERY_LENGTH: usize = 1024;
pub const MAX_SERIALIZED_RESULT_BYTES: usize = 60 * 1024; // Keep serialized JSON well under 64KB tool limit

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileMatchType {
    #[default]
    Literal,
    Glob,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub root: String,
    pub is_directory: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilesResult {
    pub matches: Vec<FileMatch>,
    pub searched_roots: Vec<String>,
    pub coverage: String,
    pub truncated: bool,
    pub limits_reached: Vec<String>,
    pub entries_inspected: usize,
    pub directories_scanned: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TextMatch {
    pub path: String,
    pub relative_path: String,
    pub root: String,
    pub line_number: usize,
    pub byte_offset: u64,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextResult {
    pub matches: Vec<TextMatch>,
    pub searched_roots: Vec<String>,
    pub coverage: String,
    pub truncated: bool,
    pub limits_reached: Vec<String>,
    pub files_scanned: usize,
    pub bytes_inspected: u64,
    pub binary_files_skipped: usize,
    pub unscanned_files_remainder: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchFilesArgs {
    pattern: String,
    #[serde(default)]
    project_root: Option<String>,
    #[serde(default)]
    match_type: Option<FileMatchType>,
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    cursor: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchTextArgs {
    query: String,
    #[serde(default)]
    project_root: Option<String>,
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[serde(default)]
    file_pattern: Option<String>,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    cursor: Option<usize>,
}

/// Matches a string against a glob pattern supporting `*` (zero or more chars) and `?` (any single char).
/// Operates iteratively without catastrophic backtracking or regex compilation.
pub fn glob_match(pattern: &str, text: &str, case_sensitive: bool) -> bool {
    let (pat, txt) = if case_sensitive {
        (pattern.to_string(), text.to_string())
    } else {
        (pattern.to_lowercase(), text.to_lowercase())
    };

    let p_chars: Vec<char> = pat.chars().collect();
    let t_chars: Vec<char> = txt.chars().collect();

    let mut p_idx = 0;
    let mut t_idx = 0;
    let mut star_idx = None;
    let mut match_idx = 0;

    while t_idx < t_chars.len() {
        if p_idx < p_chars.len() && (p_chars[p_idx] == '?' || p_chars[p_idx] == t_chars[t_idx]) {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < p_chars.len() && p_chars[p_idx] == '*' {
            star_idx = Some(p_idx);
            p_idx += 1;
            match_idx = t_idx;
        } else if let Some(s_idx) = star_idx {
            p_idx = s_idx + 1;
            match_idx += 1;
            t_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < p_chars.len() && p_chars[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == p_chars.len()
}

/// Determines whether a filename or relative path matches the requested pattern.
pub fn matches_file_pattern(
    name: &str,
    rel_path: &str,
    pattern: &str,
    match_type: FileMatchType,
    case_sensitive: bool,
) -> bool {
    let (target_norm, pat_norm);
    let (match_target, normalized_pattern) = if pattern.contains('/') || pattern.contains('\\') {
        target_norm = rel_path.replace('\\', "/");
        pat_norm = pattern.replace('\\', "/");
        (target_norm.as_str(), pat_norm.as_str())
    } else {
        (name, pattern)
    };

    match match_type {
        FileMatchType::Literal => {
            if case_sensitive {
                match_target.contains(normalized_pattern)
            } else {
                match_target
                    .to_lowercase()
                    .contains(&normalized_pattern.to_lowercase())
            }
        }
        FileMatchType::Glob => glob_match(normalized_pattern, match_target, case_sensitive),
    }
}

fn match_char_case_insensitive(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let mut a_lower = a.to_lowercase();
    let mut b_lower = b.to_lowercase();
    match (a_lower.next(), b_lower.next()) {
        (Some(c1), Some(c2)) if a_lower.next().is_none() && b_lower.next().is_none() => c1 == c2,
        _ => a.to_lowercase().to_string() == b.to_lowercase().to_string(),
    }
}

fn starts_with_case_insensitive(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let mut h_chars = haystack.chars();
    let mut n_chars = needle.chars();
    let mut matched_bytes = 0;

    loop {
        match (h_chars.next(), n_chars.next()) {
            (Some(hc), Some(nc)) => {
                if !match_char_case_insensitive(hc, nc) {
                    return None;
                }
                matched_bytes += hc.len_utf8();
            }
            (None, Some(_)) => return None,
            (_, None) => return Some((matched_bytes, needle.chars().count())),
        }
    }
}

/// Formats a short, bounded snippet surrounding a match on a single or multiline range.
fn create_bounded_snippet(
    full_text: &str,
    match_pos: usize,
    match_len: usize,
    max_line_len: usize,
    context_chars: usize,
) -> String {
    let line_start = full_text[..match_pos]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let line_end = full_text[match_pos + match_len..]
        .find(|c| c == '\n' || c == '\r')
        .map(|idx| match_pos + match_len + idx)
        .unwrap_or(full_text.len());

    let line_slice = &full_text[line_start..line_end];
    let char_count = line_slice.chars().count();

    if char_count <= max_line_len {
        line_slice.trim_matches('\r').to_string()
    } else {
        // Long line: extract context window around match
        let match_in_line_start = match_pos - line_start;
        let match_in_line_end = match_in_line_start + match_len;

        let char_indices: Vec<(usize, char)> = line_slice.char_indices().collect();
        let total_chars = char_indices.len();

        let start_char_idx = char_indices
            .iter()
            .position(|&(byte_offset, _)| byte_offset >= match_in_line_start)
            .unwrap_or(0);
        let end_char_idx = char_indices
            .iter()
            .position(|&(byte_offset, _)| byte_offset >= match_in_line_end)
            .unwrap_or(total_chars);

        let window_start_char = start_char_idx.saturating_sub(context_chars);
        let window_end_char = (end_char_idx + context_chars).min(total_chars);

        let window_start_byte = char_indices[window_start_char].0;
        let window_end_byte = if window_end_char < total_chars {
            char_indices[window_end_char].0
        } else {
            line_slice.len()
        };

        let snippet_sub = &line_slice[window_start_byte..window_end_byte];
        let mut snippet = String::new();
        if window_start_char > 0 {
            snippet.push_str("... ");
        }
        snippet.push_str(snippet_sub.trim_matches('\r'));
        if window_end_char < total_chars {
            snippet.push_str(" ...");
        }
        snippet
    }
}

enum FileScanOutcome {
    Success {
        matches: Vec<TextMatch>,
        bytes_inspected: u64,
        reached_file_eof: bool,
    },
    Binary,
}

/// Streams a file in bounded chunks searching for literal query matches.
/// Handles cross-chunk UTF-8 code point boundaries, boundary-spanning literal matches,
/// and long lines without arbitrary whole-file rejection.
fn scan_file_for_text(
    file_path: &Path,
    canon_path_str: &str,
    relative_path: &str,
    root_str: &str,
    query: &str,
    case_sensitive: bool,
    max_bytes_to_read: u64,
    mut matches_needed: usize,
    mut cursor_skip: usize,
) -> Result<FileScanOutcome, std::io::Error> {
    let started = Instant::now();
    let mut file = File::open(file_path)?;

    let mut carry_over = String::new();
    let mut file_offset_of_full_text: u64 = 0;
    let mut lines_before_full_text: usize = 1;
    let mut last_emitted_offset: Option<u64> = None;
    let mut bytes_inspected_in_file: u64 = 0;
    let mut found_matches = Vec::new();
    let mut reached_file_eof = false;

    let first_q_char = query.chars().next();

    loop {
        if matches_needed == 0 || started.elapsed() >= MAX_SEARCH_DURATION {
            break;
        }

        let remaining_budget = max_bytes_to_read.saturating_sub(bytes_inspected_in_file);
        if remaining_budget == 0 {
            break;
        }

        let to_read = (SEARCH_READ_CHUNK_SIZE as u64).min(remaining_budget) as usize;
        let mut raw_buf = vec![0u8; to_read];
        let bytes_read = file.read(&mut raw_buf)?;
        if bytes_read == 0 {
            reached_file_eof = true;
            break;
        }
        raw_buf.truncate(bytes_read);
        bytes_inspected_in_file += bytes_read as u64;

        // Detect null bytes (unsupported binary files)
        if raw_buf.contains(&0) {
            return Ok(FileScanOutcome::Binary);
        }

        // Validate UTF-8 and handle split code points across chunk boundaries
        let (valid_str, _bytes_consumed) = match std::str::from_utf8(&raw_buf) {
            Ok(s) => (s, bytes_read),
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if e.error_len().is_some() {
                    // Invalid byte sequence in stream -> treat as unsupported binary
                    return Ok(FileScanOutcome::Binary);
                }
                if bytes_read < to_read {
                    // Reached EOF inside incomplete multi-byte code point
                    return Ok(FileScanOutcome::Binary);
                }
                if valid_up_to == 0 {
                    // Buffer cut before a single code point could be completed
                    let lead = raw_buf[0];
                    let char_len: usize = if (lead & 0b1110_0000) == 0b1100_0000 {
                        2
                    } else if (lead & 0b1111_0000) == 0b1110_0000 {
                        3
                    } else if (lead & 0b1111_1000) == 0b1111_0000 {
                        4
                    } else {
                        return Ok(FileScanOutcome::Binary);
                    };
                    let needed = char_len.saturating_sub(bytes_read);
                    if needed > 0 {
                        let mut extra = vec![0u8; needed];
                        file.read_exact(&mut extra)?;
                        bytes_inspected_in_file += needed as u64;
                        raw_buf.extend_from_slice(&extra);
                    }
                    let s = match std::str::from_utf8(&raw_buf) {
                        Ok(s) => s,
                        Err(_) => return Ok(FileScanOutcome::Binary),
                    };
                    (s, raw_buf.len())
                } else {
                    let incomplete_len = bytes_read - valid_up_to;
                    file.seek(SeekFrom::Current(-(incomplete_len as i64)))?;
                    bytes_inspected_in_file -= incomplete_len as u64;
                    let s = &raw_buf[..valid_up_to];
                    let valid_str = match std::str::from_utf8(s) {
                        Ok(v) => v,
                        Err(_) => return Ok(FileScanOutcome::Binary),
                    };
                    (valid_str, valid_up_to)
                }
            }
        };

        let full_text = format!("{}{}", carry_over, valid_str);

        // Find matches in full_text
        let mut match_spans: Vec<(usize, usize)> = Vec::new(); // (byte_offset_in_full_text, match_byte_len)
        if case_sensitive {
            for (idx, _) in full_text.match_indices(query) {
                match_spans.push((idx, query.len()));
            }
        } else if let Some(first_q) = first_q_char {
            for (idx, ch) in full_text.char_indices() {
                if match_char_case_insensitive(ch, first_q) {
                    if let Some((matched_bytes, _)) =
                        starts_with_case_insensitive(&full_text[idx..], query)
                    {
                        match_spans.push((idx, matched_bytes));
                    }
                }
            }
        }

        for (m_offset, m_len) in match_spans {
            let file_byte_offset = file_offset_of_full_text + m_offset as u64;

            // Avoid duplicate emission of boundary-spanning matches from carry-over
            if let Some(last_off) = last_emitted_offset {
                if file_byte_offset <= last_off {
                    continue;
                }
            }

            if cursor_skip > 0 {
                cursor_skip -= 1;
                last_emitted_offset = Some(file_byte_offset);
                continue;
            }

            let newlines_before_match = full_text[..m_offset]
                .as_bytes()
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            let line_number = lines_before_full_text + newlines_before_match;

            let snippet = create_bounded_snippet(
                &full_text,
                m_offset,
                m_len,
                MAX_SNIPPET_LINE_LENGTH,
                MAX_SNIPPET_CONTEXT_CHARS,
            );

            found_matches.push(TextMatch {
                path: canon_path_str.to_string(),
                relative_path: relative_path.to_string(),
                root: root_str.to_string(),
                line_number,
                byte_offset: file_byte_offset,
                snippet,
            });

            last_emitted_offset = Some(file_byte_offset);
            matches_needed = matches_needed.saturating_sub(1);
            if matches_needed == 0 || started.elapsed() >= MAX_SEARCH_DURATION {
                break;
            }
        }

        // Prepare carry-over for the next chunk
        let carry_bytes_needed = (query.len().max(4) * 2).min(full_text.len());
        let mut carry_start = full_text.len().saturating_sub(carry_bytes_needed);
        while carry_start < full_text.len() && !full_text.is_char_boundary(carry_start) {
            carry_start += 1;
        }

        let newlines_before_carry = full_text[..carry_start]
            .as_bytes()
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        lines_before_full_text += newlines_before_carry;
        file_offset_of_full_text += carry_start as u64;
        carry_over = full_text[carry_start..].to_string();
    }

    Ok(FileScanOutcome::Success {
        matches: found_matches,
        bytes_inspected: bytes_inspected_in_file,
        reached_file_eof,
    })
}

/// Executes bounded, deterministic filename search across share-enabled approved roots.
pub fn search_project_files_impl(
    config_dir: &Path,
    arguments_json: &str,
) -> Result<SearchFilesResult, String> {
    let args: SearchFilesArgs = serde_json::from_str(arguments_json)
        .map_err(|e| format!("Invalid arguments for search_project_files: malformed JSON ({e})"))?;

    let trimmed_pattern = args.pattern.trim();
    if trimmed_pattern.is_empty() {
        return Err("Search pattern cannot be empty.".to_string());
    }
    if trimmed_pattern.len() > MAX_QUERY_LENGTH {
        return Err(format!(
            "Search pattern exceeds maximum allowed length ({MAX_QUERY_LENGTH} bytes)."
        ));
    }

    let match_type = args.match_type.unwrap_or_default();
    let case_sensitive = args.case_sensitive.unwrap_or(false);
    let max_depth = args
        .max_depth
        .unwrap_or(DEFAULT_MAX_DEPTH)
        .clamp(1, MAX_ALLOWED_DEPTH);
    let max_results = args
        .max_results
        .unwrap_or(DEFAULT_MAX_FILE_RESULTS)
        .clamp(1, MAX_ALLOWED_FILE_RESULTS);
    let target_cursor = args.cursor.unwrap_or(0);

    let shareable_roots = get_shareable_project_roots_impl(config_dir)?;

    let target_roots: Vec<String> = if let Some(ref specific_root) = args.project_root {
        let trimmed = specific_root.trim();
        if trimmed.is_empty() {
            return Err("project_root cannot be empty when specified.".to_string());
        }

        let mut matched = None;
        for sr in &shareable_roots {
            if sr == trimmed {
                matched = Some(sr.clone());
                break;
            }
            if let (Ok(c1), Ok(c2)) = (dunce::canonicalize(sr), dunce::canonicalize(trimmed)) {
                if c1 == c2 {
                    matched = Some(sr.clone());
                    break;
                }
            }
        }

        match matched {
            Some(r) => vec![r],
            None => {
                return Err(
                    "Access denied: Root directory is not an approved project folder or provider sharing is not enabled."
                        .to_string(),
                );
            }
        }
    } else {
        shareable_roots
    };

    if target_roots.is_empty() {
        return Ok(SearchFilesResult {
            matches: Vec::new(),
            searched_roots: Vec::new(),
            coverage: "complete".to_string(),
            truncated: false,
            limits_reached: Vec::new(),
            entries_inspected: 0,
            directories_scanned: 0,
            next_cursor: None,
        });
    }

    let start_time = Instant::now();
    let mut matches = Vec::new();
    let mut searched_roots = Vec::new();
    let mut limits_reached = HashSet::new();
    let mut total_directories_scanned = 0usize;
    let mut entries_inspected = 0usize;
    let mut cursor_skipped = 0usize;

    for root_str in target_roots {
        let canon_root = match dunce::canonicalize(&root_str) {
            Ok(c) => c,
            Err(_) => {
                limits_reached.insert("inaccessible_root".to_string());
                continue;
            }
        };

        if is_unc_or_device_path(&canon_root)
            || is_drive_root(&canon_root)
            || is_user_profile_root(&canon_root)
            || is_system_directory(&canon_root)
            || is_yuki_internal_path(config_dir, &canon_root)
        {
            continue;
        }

        searched_roots.push(root_str.clone());

        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        visited.insert(canon_root.clone());
        queue.push_back((canon_root.clone(), 0usize));

        let mut hit_stop_limit = false;

        while let Some((curr_dir, curr_depth)) = queue.pop_front() {
            if start_time.elapsed() >= MAX_SEARCH_DURATION {
                limits_reached.insert("time_limit".to_string());
                hit_stop_limit = true;
                break;
            }

            if total_directories_scanned >= MAX_DIRECTORIES_SCANNED {
                limits_reached.insert("max_directories".to_string());
                hit_stop_limit = true;
                break;
            }

            total_directories_scanned += 1;

            let read_dir = match std::fs::read_dir(&curr_dir) {
                Ok(rd) => rd,
                Err(_) => {
                    limits_reached.insert("inaccessible_directory".to_string());
                    continue;
                }
            };

            let mut dir_entries = Vec::new();
            for entry_res in read_dir {
                entries_inspected += 1;
                if entries_inspected >= MAX_ENTRIES_INSPECTED
                    || start_time.elapsed() >= MAX_SEARCH_DURATION
                {
                    limits_reached.insert("enumeration_limit".to_string());
                    hit_stop_limit = true;
                    break;
                }

                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let name = entry.file_name().to_string_lossy().to_string();
                if is_dos_device_name(&name)
                    || is_blocked_name(&name)
                    || is_secret_directory(&name)
                    || is_cache_or_generated_dir(&name)
                {
                    continue;
                }

                let entry_path = entry.path();
                if is_symlink_or_reparse(&entry_path) {
                    continue;
                }

                let canon_entry = match dunce::canonicalize(&entry_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if !canon_entry.starts_with(&canon_root) {
                    continue;
                }

                if is_system_directory(&canon_entry)
                    || is_yuki_internal_path(config_dir, &canon_entry)
                {
                    continue;
                }

                let is_dir = canon_entry.is_dir();
                dir_entries.push((name, canon_entry, is_dir));
            }

            // Sort deterministically (case-insensitive)
            dir_entries.sort_by(|a, b| {
                a.0.to_lowercase()
                    .cmp(&b.0.to_lowercase())
                    .then_with(|| a.0.cmp(&b.0))
            });

            let mut child_dirs = Vec::new();

            for (name, canon_entry, is_dir) in dir_entries {
                let rel_path_buf = canon_entry
                    .strip_prefix(&canon_root)
                    .unwrap_or(Path::new(""));
                let rel_path = rel_path_buf.to_string_lossy().replace('\\', "/");

                if matches_file_pattern(
                    &name,
                    &rel_path,
                    trimmed_pattern,
                    match_type,
                    case_sensitive,
                ) {
                    if cursor_skipped < target_cursor {
                        cursor_skipped += 1;
                    } else {
                        let size_bytes = if !is_dir {
                            std::fs::metadata(&canon_entry).ok().map(|m| m.len())
                        } else {
                            None
                        };

                        matches.push(FileMatch {
                            name: name.clone(),
                            path: canon_entry.to_string_lossy().to_string(),
                            relative_path: rel_path,
                            root: root_str.clone(),
                            is_directory: is_dir,
                            size_bytes,
                        });

                        if matches.len() >= max_results {
                            limits_reached.insert("max_results".to_string());
                            hit_stop_limit = true;
                            break;
                        }
                    }
                }

                if is_dir {
                    child_dirs.push(canon_entry);
                }
            }

            if hit_stop_limit {
                break;
            }

            if curr_depth < max_depth {
                for child_dir in child_dirs {
                    if !visited.contains(&child_dir) {
                        visited.insert(child_dir.clone());
                        queue.push_back((child_dir, curr_depth + 1));
                    }
                }
            } else if !child_dirs.is_empty() {
                limits_reached.insert("max_depth".to_string());
            }
        }

        if hit_stop_limit {
            break;
        }
    }

    let mut limits_vec: Vec<String> = limits_reached.into_iter().collect();
    limits_vec.sort();

    let truncated = !limits_vec.is_empty();
    let coverage = if truncated {
        "partial".to_string()
    } else {
        "complete".to_string()
    };

    let next_cursor = if truncated && matches.len() >= max_results {
        Some(target_cursor + matches.len())
    } else {
        None
    };

    let mut res = SearchFilesResult {
        matches,
        searched_roots,
        coverage,
        truncated,
        limits_reached: limits_vec,
        entries_inspected,
        directories_scanned: total_directories_scanned,
        next_cursor,
    };

    // Guarantee serialized output budget
    while serde_json::to_string(&res).map(|s| s.len()).unwrap_or(0) > MAX_SERIALIZED_RESULT_BYTES
        && !res.matches.is_empty()
    {
        res.matches.pop();
        res.truncated = true;
        res.coverage = "partial".to_string();
        if !res
            .limits_reached
            .contains(&"result_size_budget".to_string())
        {
            res.limits_reached.push("result_size_budget".to_string());
            res.limits_reached.sort();
        }
    }

    Ok(res)
}

/// Executes bounded, deterministic literal text search across files in share-enabled approved roots.
pub fn search_project_text_impl(
    config_dir: &Path,
    arguments_json: &str,
) -> Result<SearchTextResult, String> {
    let args: SearchTextArgs = serde_json::from_str(arguments_json)
        .map_err(|e| format!("Invalid arguments for search_project_text: malformed JSON ({e})"))?;

    let trimmed_query = args.query.trim();
    if trimmed_query.is_empty() {
        return Err("Search query cannot be empty.".to_string());
    }
    if trimmed_query.len() > MAX_QUERY_LENGTH {
        return Err(format!(
            "Search query exceeds maximum allowed length ({MAX_QUERY_LENGTH} bytes)."
        ));
    }

    let case_sensitive = args.case_sensitive.unwrap_or(false);
    let max_depth = args
        .max_depth
        .unwrap_or(DEFAULT_MAX_DEPTH)
        .clamp(1, MAX_ALLOWED_DEPTH);
    let max_results = args
        .max_results
        .unwrap_or(DEFAULT_MAX_TEXT_RESULTS)
        .clamp(1, MAX_ALLOWED_TEXT_RESULTS);
    let target_cursor = args.cursor.unwrap_or(0);
    if target_cursor != 0 {
        return Err(
            "Text search continuation is not supported; narrow the root or file pattern."
                .to_string(),
        );
    }

    let shareable_roots = get_shareable_project_roots_impl(config_dir)?;

    let target_roots: Vec<String> = if let Some(ref specific_root) = args.project_root {
        let trimmed = specific_root.trim();
        if trimmed.is_empty() {
            return Err("project_root cannot be empty when specified.".to_string());
        }

        let mut matched = None;
        for sr in &shareable_roots {
            if sr == trimmed {
                matched = Some(sr.clone());
                break;
            }
            if let (Ok(c1), Ok(c2)) = (dunce::canonicalize(sr), dunce::canonicalize(trimmed)) {
                if c1 == c2 {
                    matched = Some(sr.clone());
                    break;
                }
            }
        }

        match matched {
            Some(r) => vec![r],
            None => {
                return Err(
                    "Access denied: Root directory is not an approved project folder or provider sharing is not enabled."
                        .to_string(),
                );
            }
        }
    } else {
        shareable_roots
    };

    if target_roots.is_empty() {
        return Ok(SearchTextResult {
            matches: Vec::new(),
            searched_roots: Vec::new(),
            coverage: "complete".to_string(),
            truncated: false,
            limits_reached: Vec::new(),
            files_scanned: 0,
            bytes_inspected: 0,
            binary_files_skipped: 0,
            unscanned_files_remainder: 0,
            next_cursor: None,
        });
    }

    let start_time = Instant::now();
    let mut matches = Vec::new();
    let mut searched_roots = Vec::new();
    let mut limits_reached = HashSet::new();
    let mut total_files_scanned = 0usize;
    let mut total_bytes_inspected = 0u64;
    let mut total_binary_files_skipped = 0usize;
    let mut unscanned_remainder_files = 0usize;
    let mut cursor_skipped = 0usize;

    for root_str in target_roots {
        let canon_root = match dunce::canonicalize(&root_str) {
            Ok(c) => c,
            Err(_) => {
                limits_reached.insert("inaccessible_root".to_string());
                continue;
            }
        };

        if is_unc_or_device_path(&canon_root)
            || is_drive_root(&canon_root)
            || is_user_profile_root(&canon_root)
            || is_system_directory(&canon_root)
            || is_yuki_internal_path(config_dir, &canon_root)
        {
            continue;
        }

        searched_roots.push(root_str.clone());

        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        visited.insert(canon_root.clone());
        queue.push_back((canon_root.clone(), 0usize));

        let mut hit_stop_limit = false;

        while let Some((curr_dir, curr_depth)) = queue.pop_front() {
            if hit_stop_limit {
                break;
            }
            if visited.len() > 1000 {
                limits_reached.insert("max_directories".to_string());
                break;
            }
            if start_time.elapsed() >= MAX_SEARCH_DURATION {
                limits_reached.insert("time_limit".to_string());
                hit_stop_limit = true;
                continue;
            }

            let read_dir = match std::fs::read_dir(&curr_dir) {
                Ok(rd) => rd,
                Err(_) => {
                    limits_reached.insert("inaccessible_directory".to_string());
                    continue;
                }
            };

            let mut files = Vec::new();
            let mut child_dirs = Vec::new();

            for (entry_index, entry_res) in read_dir.enumerate() {
                if entry_index >= 20_000 || start_time.elapsed() >= MAX_SEARCH_DURATION {
                    limits_reached.insert("enumeration_limit".to_string());
                    hit_stop_limit = true;
                    break;
                }
                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let name = entry.file_name().to_string_lossy().to_string();
                if is_dos_device_name(&name)
                    || is_blocked_name(&name)
                    || is_secret_directory(&name)
                    || is_cache_or_generated_dir(&name)
                {
                    continue;
                }

                let entry_path = entry.path();
                if is_symlink_or_reparse(&entry_path) {
                    continue;
                }

                let canon_entry = match dunce::canonicalize(&entry_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if !canon_entry.starts_with(&canon_root) {
                    continue;
                }

                if is_system_directory(&canon_entry)
                    || is_yuki_internal_path(config_dir, &canon_entry)
                {
                    continue;
                }

                if canon_entry.is_file() {
                    files.push((name, canon_entry));
                } else if canon_entry.is_dir() {
                    child_dirs.push((name, canon_entry));
                }
            }

            // Sort deterministically (case-insensitive)
            files.sort_by(|a, b| {
                a.0.to_lowercase()
                    .cmp(&b.0.to_lowercase())
                    .then_with(|| a.0.cmp(&b.0))
            });
            child_dirs.sort_by(|a, b| {
                a.0.to_lowercase()
                    .cmp(&b.0.to_lowercase())
                    .then_with(|| a.0.cmp(&b.0))
            });

            for (name, file_path) in files {
                if hit_stop_limit {
                    unscanned_remainder_files += 1;
                    continue;
                }

                // Check file pattern filter if provided
                if let Some(ref pattern) = args.file_pattern {
                    if !glob_match(pattern, &name, false) {
                        continue;
                    }
                }

                if start_time.elapsed() >= MAX_SEARCH_DURATION {
                    limits_reached.insert("time_limit".to_string());
                    hit_stop_limit = true;
                    unscanned_remainder_files += 1;
                    continue;
                }

                if total_bytes_inspected >= MAX_BYTES_INSPECTED_PER_SEARCH {
                    limits_reached.insert("bytes_budget".to_string());
                    hit_stop_limit = true;
                    unscanned_remainder_files += 1;
                    continue;
                }

                let rel_path_buf = file_path.strip_prefix(&canon_root).unwrap_or(Path::new(""));
                let rel_path = rel_path_buf.to_string_lossy().replace('\\', "/");
                let canon_path_str = file_path.to_string_lossy().to_string();

                let remaining_search_bytes =
                    MAX_BYTES_INSPECTED_PER_SEARCH.saturating_sub(total_bytes_inspected);
                let needed = max_results.saturating_sub(matches.len());
                let cur_skip = target_cursor.saturating_sub(cursor_skipped);

                if total_files_scanned >= 200 {
                    limits_reached.insert("max_files".to_string());
                    hit_stop_limit = true;
                    break;
                }
                total_files_scanned += 1;

                match scan_file_for_text(
                    &file_path,
                    &canon_path_str,
                    &rel_path,
                    &root_str,
                    trimmed_query,
                    case_sensitive,
                    remaining_search_bytes,
                    needed,
                    cur_skip,
                ) {
                    Ok(FileScanOutcome::Success {
                        matches: file_matches,
                        bytes_inspected,
                        reached_file_eof,
                    }) => {
                        total_bytes_inspected += bytes_inspected;
                        cursor_skipped += cur_skip.min(target_cursor);

                        matches.extend(file_matches);

                        if !reached_file_eof {
                            limits_reached.insert("partial_file_scan".to_string());
                            hit_stop_limit = true;
                        }

                        if matches.len() >= max_results {
                            limits_reached.insert("max_results".to_string());
                            hit_stop_limit = true;
                        }
                    }
                    Ok(FileScanOutcome::Binary) => {
                        total_binary_files_skipped += 1;
                    }
                    Err(_) => {
                        unscanned_remainder_files += 1;
                    }
                }
            }

            if curr_depth < max_depth {
                for (_, child_dir) in child_dirs {
                    if !visited.contains(&child_dir) {
                        visited.insert(child_dir.clone());
                        queue.push_back((child_dir, curr_depth + 1));
                    }
                }
            } else if !child_dirs.is_empty() {
                limits_reached.insert("max_depth".to_string());
            }
        }
    }

    let mut limits_vec: Vec<String> = limits_reached.into_iter().collect();
    limits_vec.sort();

    let truncated = !limits_vec.is_empty() || unscanned_remainder_files > 0;
    let coverage = if truncated {
        "partial".to_string()
    } else {
        "complete".to_string()
    };

    let next_cursor = None;

    let mut res = SearchTextResult {
        matches,
        searched_roots,
        coverage,
        truncated,
        limits_reached: limits_vec,
        files_scanned: total_files_scanned,
        bytes_inspected: total_bytes_inspected,
        binary_files_skipped: total_binary_files_skipped,
        unscanned_files_remainder: unscanned_remainder_files,
        next_cursor,
    };

    // Guarantee serialized output budget
    while serde_json::to_string(&res).map(|s| s.len()).unwrap_or(0) > MAX_SERIALIZED_RESULT_BYTES
        && !res.matches.is_empty()
    {
        res.matches.pop();
        res.truncated = true;
        res.coverage = "partial".to_string();
        if !res
            .limits_reached
            .contains(&"result_size_budget".to_string())
        {
            res.limits_reached.push("result_size_budget".to_string());
            res.limits_reached.sort();
        }
    }

    Ok(res)
}

/// Reauthorizes file search results before transmission to ensure no revoked roots or unapproved paths leak.
pub fn reauthorize_search_files_result(config_dir: &Path, result_content: &mut String) {
    let mut parsed: SearchFilesResult = match serde_json::from_str(result_content) {
        Ok(res) => res,
        Err(_) => return,
    };

    let shareable_roots = match get_shareable_project_roots_impl(config_dir) {
        Ok(roots) => roots,
        Err(_) => {
            *result_content = serde_json::to_string(&serde_json::json!({
                "error": "Provider sharing authorization failed before sending to provider: Unable to read shareable roots.",
                "matches": [],
                "searched_roots": [],
                "coverage": "partial",
                "truncated": true,
                "limits_reached": ["revoked_grant"],
                "entries_inspected": 0,
                "directories_scanned": 0
            }))
            .unwrap();
            return;
        }
    };

    if shareable_roots.is_empty() {
        *result_content = serde_json::to_string(&serde_json::json!({
            "error": "Provider sharing authorization failed before sending to provider: Provider sharing was revoked for all scanned project roots.",
            "matches": [],
            "searched_roots": [],
            "coverage": "partial",
            "truncated": true,
            "limits_reached": ["revoked_grant"],
            "entries_inspected": 0,
            "directories_scanned": 0
        }))
        .unwrap();
        return;
    }

    let mut canonical_shareable = Vec::new();
    for sr in &shareable_roots {
        if let Ok(c) = dunce::canonicalize(sr) {
            canonical_shareable.push((sr.clone(), c));
        }
    }

    let original_match_count = parsed.matches.len();
    parsed.matches.retain(|m| {
        let m_path = Path::new(&m.path);
        let Ok(m_canon) = dunce::canonicalize(m_path) else {
            return false;
        };

        canonical_shareable.iter().any(|(sr, sc)| {
            (m.root == *sr
                || dunce::canonicalize(&m.root)
                    .map(|c| c == *sc)
                    .unwrap_or(false))
                && m_canon.starts_with(sc)
        })
    });

    parsed.searched_roots.retain(|r| {
        canonical_shareable
            .iter()
            .any(|(sr, sc)| r == sr || dunce::canonicalize(r).map(|c| c == *sc).unwrap_or(false))
    });

    if parsed.matches.len() < original_match_count {
        parsed.coverage = "partial".to_string();
        parsed.truncated = true;
        if !parsed.limits_reached.contains(&"revoked_grant".to_string()) {
            parsed.limits_reached.push("revoked_grant".to_string());
            parsed.limits_reached.sort();
        }
    }

    if parsed.searched_roots.is_empty() {
        *result_content = serde_json::to_string(&serde_json::json!({
            "error": "Provider sharing authorization failed before sending to provider: Provider sharing was revoked for all scanned project roots.",
            "matches": [],
            "searched_roots": [],
            "coverage": "partial",
            "truncated": true,
            "limits_reached": ["revoked_grant"],
            "entries_inspected": 0,
            "directories_scanned": 0
        }))
        .unwrap();
        return;
    }

    *result_content = serde_json::to_string(&parsed).unwrap_or_else(|_| {
        "{\"error\":\"Failed to serialize reauthorized search files result.\"}".to_string()
    });
}

/// Reauthorizes text search results before transmission to ensure no revoked roots, paths, or text snippets leak.
pub fn reauthorize_search_text_result(config_dir: &Path, result_content: &mut String) {
    let mut parsed: SearchTextResult = match serde_json::from_str(result_content) {
        Ok(res) => res,
        Err(_) => return,
    };

    let shareable_roots = match get_shareable_project_roots_impl(config_dir) {
        Ok(roots) => roots,
        Err(_) => {
            *result_content = serde_json::to_string(&serde_json::json!({
                "error": "Provider sharing authorization failed before sending to provider: Unable to read shareable roots.",
                "matches": [],
                "searched_roots": [],
                "coverage": "partial",
                "truncated": true,
                "limits_reached": ["revoked_grant"],
                "files_scanned": 0,
                "bytes_inspected": 0,
                "binary_files_skipped": 0,
                "unscanned_files_remainder": 0
            }))
            .unwrap();
            return;
        }
    };

    if shareable_roots.is_empty() {
        *result_content = serde_json::to_string(&serde_json::json!({
            "error": "Provider sharing authorization failed before sending to provider: Provider sharing was revoked for all scanned project roots.",
            "matches": [],
            "searched_roots": [],
            "coverage": "partial",
            "truncated": true,
            "limits_reached": ["revoked_grant"],
            "files_scanned": 0,
            "bytes_inspected": 0,
            "binary_files_skipped": 0,
            "unscanned_files_remainder": 0
        }))
        .unwrap();
        return;
    }

    let mut canonical_shareable = Vec::new();
    for sr in &shareable_roots {
        if let Ok(c) = dunce::canonicalize(sr) {
            canonical_shareable.push((sr.clone(), c));
        }
    }

    let original_match_count = parsed.matches.len();
    parsed.matches.retain(|m| {
        let m_path = Path::new(&m.path);
        let Ok(m_canon) = dunce::canonicalize(m_path) else {
            return false;
        };

        canonical_shareable.iter().any(|(sr, sc)| {
            (m.root == *sr
                || dunce::canonicalize(&m.root)
                    .map(|c| c == *sc)
                    .unwrap_or(false))
                && m_canon.starts_with(sc)
        })
    });

    parsed.searched_roots.retain(|r| {
        canonical_shareable
            .iter()
            .any(|(sr, sc)| r == sr || dunce::canonicalize(r).map(|c| c == *sc).unwrap_or(false))
    });

    if parsed.matches.len() < original_match_count {
        parsed.coverage = "partial".to_string();
        parsed.truncated = true;
        if !parsed.limits_reached.contains(&"revoked_grant".to_string()) {
            parsed.limits_reached.push("revoked_grant".to_string());
            parsed.limits_reached.sort();
        }
    }

    if parsed.searched_roots.is_empty() {
        *result_content = serde_json::to_string(&serde_json::json!({
            "error": "Provider sharing authorization failed before sending to provider: Provider sharing was revoked for all scanned project roots.",
            "matches": [],
            "searched_roots": [],
            "coverage": "partial",
            "truncated": true,
            "limits_reached": ["revoked_grant"],
            "files_scanned": 0,
            "bytes_inspected": 0,
            "binary_files_skipped": 0,
            "unscanned_files_remainder": 0
        }))
        .unwrap();
        return;
    }

    *result_content = serde_json::to_string(&parsed).unwrap_or_else(|_| {
        "{\"error\":\"Failed to serialize reauthorized search text result.\"}".to_string()
    });
}
