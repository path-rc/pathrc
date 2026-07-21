use std::fs;
use std::path::{Path, PathBuf};

pub struct PathRC {
    directory: PathBuf,
    found_files: Vec<String>,
}

impl PathRC {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            found_files: Vec::new(),
        }
    }

    pub fn search_files(&mut self, file_name: String) {
        let start_dir = Self::pathrc_start_dir();
        while let Some(path_buf) = Self::find_pathrc(&self, &file_name, start_dir.as_deref()) {
            let reached_start_dir = start_dir
                .as_deref()
                .is_some_and(|start| Self::paths_equal(path_buf.as_path(), start));
            let p_file = path_buf.to_string_lossy() + "/" + file_name.as_str();
            self.found_files.push(p_file.to_string());

            if reached_start_dir {
                break;
            }

            let p_dir = Self::next_dir(path_buf.as_path());
            if p_dir == None {
                break;
            } else {
                self.directory = p_dir.unwrap();
            }
        }
    }

    fn next_dir(current_dir: &Path) -> Option<PathBuf> {
        if current_dir == PathBuf::from("/").as_path() {
            None
        } else {
            Some(PathBuf::from(current_dir.parent().unwrap()))
        }
    }

    fn find_pathrc(&self, file_name: &String, start_dir: Option<&Path>) -> Option<PathBuf> {
        let dir_path = self.directory.as_path();
        let mut path: PathBuf = dir_path.into();
        let file = Path::new(file_name);
        loop {
            path.push(file);
            if path.is_file() {
                // if args list then print file name
                let path_dir = path.parent();
                let buf = PathBuf::from(path_dir.unwrap());
                break Some(buf);
            }
            if let Some(start) = start_dir {
                if path.parent().is_some_and(|parent| Self::paths_equal(parent, start)) {
                    break None;
                }
            }
            if !(path.pop() && path.pop()) {
                // remove file && remove parent
                break None;
            }
        }
    }

    fn pathrc_start_dir() -> Option<PathBuf> {
        let start = envmnt::get_or("PATHRC_START", "");
        if start.is_empty() {
            None
        } else {
            Some(PathBuf::from(start))
        }
    }

    fn paths_equal(first: &Path, second: &Path) -> bool {
        if first == second {
            return true;
        }

        match (fs::canonicalize(first), fs::canonicalize(second)) {
            (Ok(first_canonical), Ok(second_canonical)) => first_canonical == second_canonical,
            _ => false,
        }
    }

    pub fn print_file_names(&mut self) {
        if self.found_files.len() == 0 {
            println!("no configuration files found in path")
            }
        else {
        println!("\npathrc file processing order:\n");
        while let Some(top) = self.found_files.pop() {
            println!("{}", top.to_string());
            }
        }
    }

    pub fn print_files_contents(&mut self) {
        while let Some(top) = self.found_files.pop() {
            let filename = top.to_string();
            let err_message = format!("Unable to read {}", filename);
            println!("# File: {}", filename);
            let contents = fs::read_to_string(filename).expect(err_message.as_str());
            print!("{contents}");
        }
    }
}
