use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const LABELS: [&str; 4] = [
    "SSH-Host / Alias",
    "Factory-URL auf dem Runner",
    "Runner-Journal (absoluter Pfad)",
    "OpenCode-Service-Registrierung (optional)",
];

/// Local connection settings only; runner identity is pinned when opening the form.
pub struct Dialog {
    runner: RunnerId,
    path: PathBuf,
    original: Option<Vec<u8>>,
    profiles: BTreeMap<RunnerId, Profile>,
    fields: [String; 4],
    focus: usize,
    cursor: usize,
    message: String,
}
fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "SSH-Konfiguration muss eine regulaere Datei sein"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "SSH-Konfiguration ist zu gross");
    Ok(Some(bytes))
}
impl Dialog {
    pub fn new(runner: RunnerId) -> Result<Self> {
        Self::load(super::config_path(), runner)
    }
    fn load(path: PathBuf, runner: RunnerId) -> Result<Self> {
        let original = read(&path)?;
        let profiles: BTreeMap<RunnerId, Profile> = original
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        let p = profiles.get(&runner);
        let fields = [
            p.map_or("", |p| p.host.as_str()).into(),
            p.map_or("", |p| p.server.as_str()).into(),
            p.map_or("", |p| p.journal.as_str()).into(),
            p.and_then(|p| p.opencode_registration.as_deref())
                .unwrap_or("")
                .into(),
        ];
        Ok(Self {runner, path, original, profiles, fields, focus: 0, cursor: 0, message: "Vorhandenen SSH-Alias verwenden. Anmeldung und Hostschluessel verwaltet dein SSH-Client.".into()})
    }
    pub fn insert(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if self.fields[self.focus].len() + text.len() > 4096 {
            self.message = "Feld maximal 4096 Bytes".into();
            return;
        }
        self.fields[self.focus].insert_str(self.cursor, &text);
        self.cursor += text.len();
    }
    fn save(&mut self, remove: bool) -> Result<()> {
        let mut next = self.profiles.clone();
        if remove {
            next.remove(&self.runner);
        } else {
            let p = Profile {
                host: self.fields[0].trim().into(),
                server: self.fields[1].trim().into(),
                journal: self.fields[2].trim().into(),
                opencode_registration: (!self.fields[3].trim().is_empty())
                    .then(|| self.fields[3].trim().into()),
            };
            super::command(&p, self.runner, CommandId::new())?;
            ensure!(
                p.journal.starts_with('/'),
                "Runner-Journal muss ein absoluter Linux-Pfad sein"
            );
            ensure!(
                p.opencode_registration
                    .as_ref()
                    .is_none_or(|s| s.starts_with('/')),
                "Registrierung muss ein absoluter Linux-Pfad sein"
            );
            next.insert(self.runner, p);
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let lock_path = self.path.with_extension("json.lock");
        if let Ok(meta) = std::fs::symlink_metadata(&lock_path) {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Ungueltige SSH-Konfigurationssperre"
            );
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.try_lock()
            .context("SSH-Konfiguration wird gerade in einer anderen TUI bearbeitet")?;
        ensure!(read(&self.path)? == self.original, "SSH-Konfiguration wurde extern geaendert. Ctrl-R: neu laden; Entwurf wird dabei verworfen.");
        let bytes = serde_json::to_vec_pretty(&next)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|e| e.error)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        self.profiles = next;
        self.original = Some(bytes);
        if remove {
            self.fields = Default::default();
            self.cursor = 0;
        }
        self.message = if remove {
            "SSH-Verbindung entfernt"
        } else {
            "SSH-Verbindung gespeichert. Session mit Enter oeffnen."
        }
        .into();
        Ok(())
    }
    /// Returns true only when closing the local dialog.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let result = match key.code {
                KeyCode::Char('s') => self.save(false),
                KeyCode::Char('d') => self.save(true),
                KeyCode::Char('r') => {
                    Self::load(self.path.clone(), self.runner).map(|new| *self = new)
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                self.message = format!("{e:#}");
            }
            return false;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.focus = (self.focus + 1) % 4;
                self.cursor = self.fields[self.focus].len();
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + 3) % 4;
                self.cursor = self.fields[self.focus].len();
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.fields[self.focus].len(),
            KeyCode::Left => {
                self.cursor = self.fields[self.focus][..self.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i)
            }
            KeyCode::Right => {
                self.cursor += self.fields[self.focus][self.cursor..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
            }
            KeyCode::Backspace => {
                let start = self.fields[self.focus][..self.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i);
                self.fields[self.focus].replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            KeyCode::Delete => {
                let end = self.cursor
                    + self.fields[self.focus][self.cursor..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8);
                self.fields[self.focus].replace_range(self.cursor..end, "");
            }
            KeyCode::Char(c) => self.insert(&c.to_string()),
            _ => {}
        }
        false
    }
    pub fn draw(&self, frame: &mut Frame) {
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(5),
        ])
        .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!("Runner {}\n{}", self.runner, self.path.display()))
                .block(Block::bordered().title(" Lokale SSH-Verbindung ")),
            areas[0],
        );
        let mut text = String::new();
        for (i, label) in LABELS.iter().enumerate() {
            text.push_str(&format!(
                "{} {}\n",
                if i == self.focus { "▶" } else { " " },
                label
            ));
            let field = &self.fields[i];
            if i == self.focus {
                let max = (frame.area().width.saturating_sub(8) as usize / 4).max(1);
                let before = field[..self.cursor]
                    .chars()
                    .rev()
                    .take(max)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>();
                let after = field[self.cursor..].chars().take(max).collect::<String>();
                text.push_str(&format!("  {before}▏{after}\n\n"));
            } else {
                text.push_str(&format!(
                    "  {}\n\n",
                    field.chars().take(120).collect::<String>()
                ));
            }
        }
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), areas[1]);
        frame.render_widget(Paragraph::new(format!("Tab/↑/↓: Feld | Ctrl-S: speichern | Ctrl-D: entfernen\nCtrl-R: neu laden | Esc: schliessen\n{}",self.message)).wrap(Wrap{trim:false}).block(Block::bordered().title(" Bedienung ")),areas[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn filled(path: PathBuf, runner: RunnerId) -> Dialog {
        let mut d = Dialog::load(path, runner).unwrap();
        d.fields = [
            "my-host".into(),
            "http://127.0.0.1:7432".into(),
            "/home/me/journal.db".into(),
            String::new(),
        ];
        d
    }
    #[test]
    fn concurrent_edit_is_rejected_and_unrelated_runner_profiles_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ssh.json");
        let a = RunnerId::new();
        let b = RunnerId::new();
        let mut first = filled(path.clone(), a);
        let mut stale = filled(path.clone(), b);
        first.save(false).unwrap();
        assert!(stale.save(false).is_err());
        let mut fresh = filled(path.clone(), b);
        fresh.save(false).unwrap();
        let mut first = Dialog::load(path.clone(), a).unwrap();
        assert_eq!(first.profiles.len(), 2);
        first.save(true).unwrap();
        let loaded = Dialog::load(path, b).unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert!(loaded.profiles.contains_key(&b));
    }
    #[test]
    fn invalid_fields_and_broken_config_never_overwrite_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ssh.json");
        let mut d = filled(path.clone(), RunnerId::new());
        d.fields[0] = "-oProxyCommand=bad".into();
        assert!(d.save(false).is_err());
        assert!(!path.exists());
        std::fs::write(&path, b"invalid").unwrap();
        assert!(Dialog::load(path.clone(), RunnerId::new()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"invalid");
    }
}
