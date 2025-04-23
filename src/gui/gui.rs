use crate::ripgrep::ripgrep::{run_ripgrep, GuiMatch, SearchResult}; // Import GuiMatch instead of Match
use crossbeam_channel::{unbounded, Receiver, TryRecvError}; // Removed Sender
use directories::UserDirs;
use std::thread; // Removed PathBuf

pub struct MyApp {
    query: String,
    path: String,
    results: Vec<GuiMatch>, // Use GuiMatch here
    error_message: Option<String>,
    search_status: String,
    // Channel for receiving results from the search thread
    search_result_receiver: Option<Receiver<SearchResult>>,
    // Options for ripgrep
    case_insensitive: bool,
    search_hidden: bool,
    follow_symlinks: bool,
    globs: String,
}

impl Default for MyApp {
    fn default() -> Self {
        let initial_path = UserDirs::new()
            .and_then(|ud| ud.home_dir().to_str().map(String::from))
            .unwrap_or_else(|| ".".to_string());

        MyApp {
            query: String::new(),
            path: initial_path,
            results: Vec::new(),
            error_message: None,
            search_status: "Ready".to_string(),
            search_result_receiver: None,
            case_insensitive: false,
            search_hidden: false,
            follow_symlinks: false,
            globs: String::new(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for results from the search thread
        if let Some(rx) = &self.search_result_receiver {
            match rx.try_recv() {
                Ok(search_result) => match search_result {
                    SearchResult::Match(gui_match) => { // Use gui_match here
                        self.results.push(gui_match); // Push GuiMatch
                        self.search_status = format!("Found {} results...", self.results.len());
                    }
                    SearchResult::Done => {
                        self.search_status = format!("Search finished. Found {} results.", self.results.len());
                        self.search_result_receiver = None; // Search is done
                    }
                    SearchResult::Error(e) => {
                        self.error_message = Some(e.clone());
                        self.search_status = format!("Search failed: {}", e);
                        self.search_result_receiver = None; // Search is done (with error)
                    }
                },
                Err(TryRecvError::Empty) => {
                    // Still searching or waiting
                    self.search_status = format!("Searching... Found {} results.", self.results.len());
                }
                Err(TryRecvError::Disconnected) => {
                    // This happens if the sender is dropped, e.g., thread panicked
                    self.error_message = Some("Search thread disconnected unexpectedly.".to_string());
                    self.search_status = "Error: Search thread disconnected.".to_string();
                    self.search_result_receiver = None;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Ripgrep GUI");
            ui.separator();

            // Search Inputs
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut self.query);
            });
            ui.horizontal(|ui| {
                ui.label("Path:");
                ui.text_edit_singleline(&mut self.path);
                if ui.button("Browse...").clicked() {
                    // Basic folder picker - consider using rfd crate for native dialogs
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                         self.path = path.display().to_string();
                    }
                }
            });

            // Ripgrep Options
            ui.collapsing("Options", |ui| {
                 ui.checkbox(&mut self.case_insensitive, "Case Insensitive (-i)");
                 ui.checkbox(&mut self.search_hidden, "Search Hidden Files (--hidden)");
                 ui.checkbox(&mut self.follow_symlinks, "Follow Symlinks (-L)");
                 ui.horizontal(|ui| {
                    ui.label("Globs (-g):");
                    // Apply hint_text directly to the TextEdit widget
                    let _response = ui.add(egui::TextEdit::singleline(&mut self.globs).hint_text("e.g., !*.log"));
                 });
            });
            ui.separator();


            // Search Button and Status
            ui.horizontal(|ui|{
                if ui.button("Search").clicked() && self.search_result_receiver.is_none() {
                    self.results.clear();
                    self.error_message = None;
                    self.search_status = "Starting search...".to_string();

                    let (tx, rx) = unbounded::<SearchResult>();
                    self.search_result_receiver = Some(rx);

                    let query = self.query.clone();
                    let path = self.path.clone();
                    let options = crate::ripgrep::ripgrep::RgOptions {
                        case_insensitive: self.case_insensitive,
                        search_hidden: self.search_hidden,
                        follow_symlinks: self.follow_symlinks,
                        globs: if self.globs.is_empty() { None } else { Some(self.globs.clone()) },
                    };

                    // Spawn a thread to run ripgrep
                    thread::spawn(move || {
                        run_ripgrep(query, path, options, tx);
                    });
                }
                 ui.label(&self.search_status);
            });


            // Display Error Message
            if let Some(err) = &self.error_message {
                ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
            }
            ui.separator();

            // Results Area
            ui.heading("Results");
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.results.is_empty() && self.error_message.is_none() && self.search_result_receiver.is_none() {
                     ui.label("No results yet. Enter a query and path, then click Search.");
                } else {
                    for m in &self.results { // m is now a GuiMatch
                        ui.group(|ui| {
                             ui.strong(format!("{}:{}", m.path, m.line_number)); // Access fields of GuiMatch
                             ui.monospace(&m.line_text); // Access fields of GuiMatch
                        });
                    }
                }
            });
        });

        // Request repaint continuously while searching to check the channel
        if self.search_result_receiver.is_some() {
             ctx.request_repaint();
        }
    }
}
