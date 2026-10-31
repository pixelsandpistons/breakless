//! The two files on disk: the followed channels and the log of advert breaks.

use std::path::{Path, PathBuf};

use serde_json::Value;

const ADVERTS_ROTATE_AT: u64 = 1_000_000;

/// `follows.json` and `adverts.jsonl` under one folder.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The folder must exist; the files are made on first write.
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// The followed channels, valid names only. An unreadable file is an empty list.
    pub fn follows(&self) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(self.dir.join("follows.json")) else {
            return Vec::new();
        };
        let Ok(list) = serde_json::from_str::<Vec<Value>>(&text) else {
            return Vec::new();
        };
        list.into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .filter(|l| valid_login(l))
            .collect()
    }

    /// Adds or removes one channel and writes the list atomically.
    pub fn set_follow(&self, login: &str, on: bool) -> std::io::Result<Vec<String>> {
        let mut list: Vec<String> = self.follows().into_iter().filter(|l| l != login).collect();
        if on {
            list.push(login.to_string());
        }
        let tmp = self.dir.join("follows.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&list).unwrap_or_default())?;
        std::fs::rename(tmp, self.dir.join("follows.json"))?;
        Ok(list)
    }

    /// One line per advert break, so anyone can see what got through where. Rotates at 1 MB.
    pub fn record(&self, mut event: Value) {
        event["at"] = Value::String(crate::twitch::now_text());
        let path = self.dir.join("adverts.jsonl");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > ADVERTS_ROTATE_AT) {
            let _ = std::fs::rename(&path, self.dir.join("adverts.jsonl.1"));
        }
        let line = format!("{event}\n");
        let written = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        if let Err(e) = written {
            tracing::warn!("cannot write adverts.jsonl: {e}");
        }
    }
}

pub use breakless_formats::gql::valid_login;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_round_trip_and_reject_bad_names() {
        let dir =
            std::env::temp_dir().join(format!("breakless-store-{}", crate::twitch::random_u64()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::new(&dir);
        assert_eq!(s.follows(), Vec::<String>::new());
        assert_eq!(s.set_follow("alpha", true).unwrap(), ["alpha"]);
        assert_eq!(s.set_follow("beta", true).unwrap(), ["alpha", "beta"]);
        assert_eq!(s.set_follow("alpha", false).unwrap(), ["beta"]);
        std::fs::write(
            dir.join("follows.json"),
            r#"["ok_name", "bad name", 5, "x"]"#,
        )
        .unwrap();
        assert_eq!(s.follows(), ["ok_name"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn records_append_lines_with_a_time() {
        let dir =
            std::env::temp_dir().join(format!("breakless-store-{}", crate::twitch::random_u64()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::new(&dir);
        s.record(serde_json::json!({"channel": "a", "from": "home"}));
        s.record(serde_json::json!({"channel": "b", "from": "exit"}));
        let text = std::fs::read_to_string(dir.join("adverts.jsonl")).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["from"], "exit");
        assert_eq!(lines[0]["at"].as_str().unwrap().len(), 19);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
