use crossbeam_channel::Sender;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

// Represents different types of messages ripgrep emits with --json
#[derive(Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // Allow unused variants/fields as they are part of the external format
enum RgJsonItem {
    Begin(Begin),
    Match(Match),
    End(End),
    Context(Context), // Added Context type
    Summary(Summary), // Added Summary type
}

// Structs for deserializing ripgrep JSON output
#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct Begin {
    path: Option<PathData>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct Match {
    pub path: PathData,
    pub lines: TextData,
    pub line_number: Option<u64>,
    pub absolute_offset: u64,
    submatches: Vec<SubMatch>,
}

// Helper struct to handle both text and bytes for path/lines
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum TextOrBytes {
    Text(String),
    Bytes(Vec<u8>),
}

impl TextOrBytes {
    fn to_string_lossy(&self) -> String {
        match self {
            TextOrBytes::Text(s) => s.clone(),
            TextOrBytes::Bytes(b) => String::from_utf8_lossy(b).to_string(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct PathData {
    #[serde(flatten)]
    text_or_bytes: TextOrBytes,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TextData {
     #[serde(flatten)]
    text_or_bytes: TextOrBytes,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct SubMatch {
    #[serde(rename = "match")]
    m: TextData,
    start: usize,
    end: usize,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct End {
    path: Option<PathData>,
    binary_offset: Option<u64>,
    stats: Stats,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct Context { // Added Context struct
    pub path: PathData,
    pub lines: TextData,
    pub line_number: Option<u64>,
    pub absolute_offset: u64,
    submatches: Vec<SubMatch>, // Context can also have submatches
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct Summary { // Added Summary struct
    elapsed_total: DurationData,
    stats: Stats,
}


#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct Stats {
    elapsed: DurationData,
    searches: u64,
    searches_with_match: u64,
    bytes_searched: u64,
    bytes_printed: u64,
    matched_lines: u64,
    matches: u64,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] // Allow unused fields
pub struct DurationData {
    secs: u64,
    nanos: u32,
    human: String,
}


// Simplified structure to send back to the GUI thread
#[derive(Debug, Clone)]
pub struct GuiMatch { // Renamed from Match
    pub path: String,
    pub line_number: u64,
    pub line_text: String,
}

// Enum to wrap results or errors sent over the channel
pub enum SearchResult {
    Match(GuiMatch), // Updated to use GuiMatch
    Error(String),
    Done,
}

// Options for configuring the ripgrep command
#[derive(Debug, Clone)]
pub struct RgOptions {
     pub case_insensitive: bool,
     pub search_hidden: bool,
     pub follow_symlinks: bool,
     pub globs: Option<String>,
}


// Function to run ripgrep and send results back through the channel
pub fn run_ripgrep(query: String, path: String, options: RgOptions, sender: Sender<SearchResult>) {
    let mut cmd_args = vec![
        "--json".to_string(),
        query, // The search pattern
        path,  // The path to search
    ];

    // Add optional arguments
    if options.case_insensitive {
        cmd_args.push("-i".to_string());
    }
    if options.search_hidden {
        cmd_args.push("--hidden".to_string());
    }
     if options.follow_symlinks {
        cmd_args.push("-L".to_string());
    }
    if let Some(globs) = options.globs {
        // Ripgrep expects multiple -g flags, not a single comma-separated string
        // Simple split by common delimiters for now. Robust parsing might be needed.
        for glob in globs.split(|c| c == ',' || c == ';') {
             let trimmed_glob = glob.trim();
             if !trimmed_glob.is_empty() {
                cmd_args.push("-g".to_string());
                cmd_args.push(trimmed_glob.to_string());
             }
        }
    }


    let child = Command::new("rg")
        .args(&cmd_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()) // Capture stderr as well
        .spawn();

    match child {
        Ok(mut child) => {
            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line_result in reader.lines() {
                    match line_result {
                        Ok(line) => {
                            match serde_json::from_str::<RgJsonItem>(&line) {
                                Ok(RgJsonItem::Match(m)) => {
                                    // Create GuiMatch from RgJsonItem::Match
                                    let gui_match = GuiMatch {
                                        path: m.path.text_or_bytes.to_string_lossy(),
                                        line_number: m.line_number.unwrap_or(0), // Handle potential missing line number
                                        line_text: m.lines.text_or_bytes.to_string_lossy().trim_end().to_string(), // Access correctly and trim
                                    };
                                    if sender.send(SearchResult::Match(gui_match)).is_err() {
                                        eprintln!("GUI channel closed, stopping search thread.");
                                        break; // Stop processing if receiver is dropped
                                    }
                                }
                                Ok(RgJsonItem::Begin(_)) | Ok(RgJsonItem::End(_)) | Ok(RgJsonItem::Context(_)) | Ok(RgJsonItem::Summary(_)) => {
                                    // Optionally handle these messages, e.g., for progress or stats
                                }
                                Err(e) => {
                                     eprintln!("Failed to parse rg JSON line: {}, line: {}", e, line);
                                     // Optionally send a specific parse error back
                                     // sender.send(SearchResult::Error(format!("JSON parse error: {}", e))).ok();
                                }
                            }
                        }
                        Err(e) => {
                            sender.send(SearchResult::Error(format!("Error reading rg output: {}", e))).ok();
                            break;
                        }
                    }
                }
            } else {
                 sender.send(SearchResult::Error("Failed to capture rg stdout.".to_string())).ok();
            }

            // Check rg exit status and stderr
            match child.wait_with_output() {
                 Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        // Avoid sending duplicate error if already sent one
                        if !stderr.is_empty() {
                             sender.send(SearchResult::Error(format!("rg exited with error: {}", stderr.trim()))).ok();
                        } else if output.status.code().is_some() {
                             sender.send(SearchResult::Error(format!("rg exited with status: {}", output.status))).ok();
                        } else {
                             sender.send(SearchResult::Error("rg exited with non-zero status.".to_string())).ok();
                        }
                    } else {
                         // Send Done signal only if rg finished successfully
                         sender.send(SearchResult::Done).ok();
                    }
                 }
                 Err(e) => {
                     sender.send(SearchResult::Error(format!("Failed to wait for rg process: {}", e))).ok();
                 }
            }

        }
        Err(e) => {
            let err_msg = if e.kind() == std::io::ErrorKind::NotFound {
                "Error: 'rg' command not found. Please ensure ripgrep is installed and in your PATH.".to_string()
            } else {
                format!("Failed to spawn rg process: {}", e)
            };
            sender.send(SearchResult::Error(err_msg)).ok();
            // Also send Done because the process never started, so it's effectively "done" failing.
            // Or maybe just Error is sufficient. Let's stick to just Error.
            // sender.send(SearchResult::Done).ok();
        }
    }
    // Sender is automatically dropped when the thread exits, signaling disconnection if the receiver checks.
}
