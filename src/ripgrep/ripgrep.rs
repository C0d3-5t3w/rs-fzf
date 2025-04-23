use crossbeam_channel::Sender;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};


#[derive(Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] 
enum RgJsonItem {
    Begin(Begin),
    Match(Match),
    End(End),
    Context(Context), 
    Summary(Summary), 
}


#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] 
pub struct Begin {
    path: Option<PathData>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] 
pub struct Match {
    pub path: PathData,
    pub lines: TextData,
    pub line_number: Option<u64>,
    pub absolute_offset: u64,
    submatches: Vec<SubMatch>,
}


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
#[allow(dead_code)] 
pub struct SubMatch {
    #[serde(rename = "match")]
    m: TextData,
    start: usize,
    end: usize,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] 
pub struct End {
    path: Option<PathData>,
    binary_offset: Option<u64>,
    stats: Stats,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] 
pub struct Context { 
    pub path: PathData,
    pub lines: TextData,
    pub line_number: Option<u64>,
    pub absolute_offset: u64,
    submatches: Vec<SubMatch>, 
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] 
pub struct Summary { 
    elapsed_total: DurationData,
    stats: Stats,
}


#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)] 
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
#[allow(dead_code)] 
pub struct DurationData {
    secs: u64,
    nanos: u32,
    human: String,
}



#[derive(Debug, Clone)]
pub struct GuiMatch { 
    pub path: String,
    pub line_number: u64,
    pub line_text: String,
}


pub enum SearchResult {
    Match(GuiMatch), 
    Error(String),
    Done,
}


#[derive(Debug, Clone)]
pub struct RgOptions {
     pub case_insensitive: bool,
     pub search_hidden: bool,
     pub follow_symlinks: bool,
     pub globs: Option<String>,
}



pub fn run_ripgrep(query: String, path: String, options: RgOptions, sender: Sender<SearchResult>) {
    let mut cmd_args = vec![
        "--json".to_string(),
        query, 
        path,  
    ];

    
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
        .stderr(Stdio::piped()) 
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
                                    
                                    let gui_match = GuiMatch {
                                        path: m.path.text_or_bytes.to_string_lossy(),
                                        line_number: m.line_number.unwrap_or(0), 
                                        line_text: m.lines.text_or_bytes.to_string_lossy().trim_end().to_string(), 
                                    };
                                    if sender.send(SearchResult::Match(gui_match)).is_err() {
                                        eprintln!("GUI channel closed, stopping search thread.");
                                        break; 
                                    }
                                }
                                Ok(RgJsonItem::Begin(_)) | Ok(RgJsonItem::End(_)) | Ok(RgJsonItem::Context(_)) | Ok(RgJsonItem::Summary(_)) => {
                                    
                                }
                                Err(e) => {
                                     eprintln!("Failed to parse rg JSON line: {}, line: {}", e, line);
                                     
                                     
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

            
            match child.wait_with_output() {
                 Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        
                        if !stderr.is_empty() {
                             sender.send(SearchResult::Error(format!("rg exited with error: {}", stderr.trim()))).ok();
                        } else if output.status.code().is_some() {
                             sender.send(SearchResult::Error(format!("rg exited with status: {}", output.status))).ok();
                        } else {
                             sender.send(SearchResult::Error("rg exited with non-zero status.".to_string())).ok();
                        }
                    } else {
                         
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
        }
    }
    
}
