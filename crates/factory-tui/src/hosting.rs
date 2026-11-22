use anyhow::{ensure, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use ratatui::{
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

#[derive(Clone)]
pub struct Publication {
    runner: RunnerId,
    command: CommandId,
    server: String,
    journal: String,
    config: String,
    token_file: String,
    remote: bool,
}
impl Publication {
    pub async fn run(self) -> Result<String> {
        let command = if self.remote {
            crate::ssh::publish_review(self.runner, self.command, &self.config, &self.token_file)?
                .ok_or_else(|| anyhow::anyhow!("SSH-Profil wurde entfernt; kein lokaler Fallback"))?
        } else {
            ensure!(
                crate::ssh::configured(self.runner, self.command)?.is_none(),
                "SSH-Zuordnung geaendert; Dialog erneut oeffnen"
            );
            let mut command = std::process::Command::new("factory-runner");
            command.args([
                "--id",
                &self.runner.to_string(),
                "--name",
                "hosting-tui",
                "--server",
                &self.server,
                "--journal",
                &self.journal,
                "--publish-review",
                &self.command.to_string(),
                "--hosting-config",
                &self.config,
            ]);
            if !self.token_file.is_empty() {
                command
                    .env_remove("FACTORY_TOKEN")
                    .env("FACTORY_TOKEN_FILE", &self.token_file);
            }
            command
        };
        crate::attach::remote_report_timeout(command, 180).await
    }
}

pub struct Dialog {
    publication: Publication,
    commit: String,
    fields: [String; 3],
    focus: usize,
    message: String,
}
pub enum Action {
    None,
    Close,
    Publish(Publication),
}
impl Dialog {
    pub fn new(command: &RunnerCommand, server: &str) -> Result<Self> {
        ensure!(
            command.kind == CommandKind::ReviewCheckpoint
                && command.state == CommandState::Succeeded,
            "Zuerst einen erfolgreichen Review auswaehlen"
        );
        let Some(CommandOutput::Review { review }) = &command.result else {
            anyhow::bail!("Review-Ergebnis fehlt")
        };
        ensure!(
            review.test_exit_code == 0,
            "Review-Tests sind nicht erfolgreich"
        );
        let remote = crate::ssh::configured(command.runner_id, command.id)?.is_some();
        Ok(Self {
            publication: Publication {
                runner: command.runner_id,
                command: command.id,
                server: server.into(),
                journal: String::new(),
                config: String::new(),
                token_file: String::new(),
                remote,
            },
            commit: review.candidate_commit.clone(),
            fields: [String::new(), String::new(), String::new()],
            focus: if remote { 1 } else { 0 },
            message: String::new(),
        })
    }
    pub fn insert(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.fields[self.focus].len() + c.len_utf8() <= 4096 {
                self.fields[self.focus].push(c);
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Tab | KeyCode::Down => {
                self.focus = if self.focus == 2 {
                    if self.publication.remote {
                        1
                    } else {
                        0
                    }
                } else {
                    self.focus + 1
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = if self.focus == 0 || (self.publication.remote && self.focus == 1) {
                    2
                } else {
                    self.focus - 1
                }
            }
            KeyCode::Backspace => {
                self.fields[self.focus].pop();
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.fields[1].is_empty()
                    || (!self.publication.remote && self.fields[0].is_empty())
                    || (self.publication.remote && self.fields[2].is_empty())
                {
                    self.message="Hosting-Konfiguration und Journal (lokal) bzw. Factory-Tokendatei (SSH) angeben".into();
                    return Action::None;
                }
                if self
                    .fields
                    .iter()
                    .any(|v| !v.is_empty() && !std::path::Path::new(v).is_absolute())
                {
                    self.message = "Absolute Pfade auf dem ausfuehrenden Rechner angeben".into();
                    return Action::None;
                }
                let mut publication = self.publication.clone();
                publication.journal = self.fields[0].clone();
                publication.config = self.fields[1].clone();
                publication.token_file = self.fields[2].clone();
                return Action::Publish(publication);
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            _ => {}
        }
        Action::None
    }
    pub fn render(&self, frame: &mut Frame) {
        let mut text=format!("Review: {}\nKandidat: {}\nAusfuehrung: {}\n\nGeprueften Commit auf einen eigenen Branch pushen und Pull Request erstellen/abgleichen. Ziel und Zugang stehen in der Hosting-Konfiguration auf dem Runner.\n\n",self.publication.command,self.commit,if self.publication.remote {"SSH-Profil des Runners"}else{"lokaler factory-runner"});
        for (i, label) in [
            "Runner-Journal (bei SSH aus Profil)",
            "Hosting-Konfiguration JSON",
            "Factory-Operator-Tokendatei (SSH erforderlich; lokal optional)",
        ]
        .iter()
        .enumerate()
        {
            text.push_str(&format!(
                "{} {label}: {}\n",
                if self.focus == i { ">" } else { " " },
                self.fields[i]
            ));
        }
        text.push_str(&format!("\n{}\n\nTab: Feld | Ctrl-S: veroeffentlichen/abgleichen | Esc: zurueck\nBei Antwortverlust denselben Review mit derselben Konfiguration erneut waehlen; das Publikationsjournal verhindert doppelte PRs.",self.message));
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(" Hosting-Publikation ")),
            frame.area(),
        );
    }
}
