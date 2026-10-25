//! Device-suite commands operate on the real controller and its isolated Store.
use super::*;
use anyhow::Context;
use serde_json::{Value, json};

impl Controller {
    pub(super) fn playback_test(&mut self, request: Value) -> Result<Value> {
        let number = |key: &str| request[key].as_u64().unwrap_or(0);
        let command = match request["op"].as_str().context("Missing test operation")? {
            "fixture" => {
                self.save()?;
                let files: Vec<Media> = serde_json::from_value(request["files"].clone())?;
                let root = request["root"].as_str().context("Missing root")?;
                let mut library = self.store()?.import(vec![Draft {
                    root_uri: root.into(),
                    title: request["title"].as_str().context("Missing title")?.into(),
                    author: "Device suite".into(),
                    files,
                    include: true,
                }])?;
                if request["folder"].as_bool().unwrap_or(false) {
                    self.store()?.ensure_folder(root)?;
                    library = self.store()?.load()?;
                }
                self.reconcile(library);
                None
            }
            "state" => None,
            "sync_prepare" => {
                let path = std::path::PathBuf::from(
                    request["path"].as_str().context("Missing sync fixture")?,
                );
                let library = self.store()?.import(vec![Draft {
                    root_uri: file_uri(path.parent().context("Missing fixture directory")?)?,
                    title: "Sync fixture".into(),
                    author: "Device suite".into(),
                    include: true,
                    files: vec![Media {
                        uri: file_uri(&path)?,
                        identity: "sync-fixture".into(),
                        relative: "book.wav".into(),
                        size: Some(std::fs::metadata(&path)?.len()),
                        modified: Some(1),
                        duration: Some(120_000),
                        title: "Sync fixture".into(),
                        ..Default::default()
                    }],
                }])?;
                self.reconcile(library);
                let mut settings = self.options.settings.clone();
                settings.local_sync = true;
                Some(Command::Settings(settings))
            }
            "sync_progress" => {
                let part = self.library.parts.first().context("Missing sync book")?.id;
                if self.playback.phase == crate::player::Phase::Ready {
                    self.seek_to(number("position"));
                } else {
                    self.library.session.current = Some(Target::Book(part));
                    self.library.update_progress(number("position"), false);
                    self.save()?;
                }
                None
            }
            "sync_reset" => Some(Command::ResetProgress(
                self.library.books.first().context("Missing sync book")?.id,
            )),
            "sync_open" => {
                let part = self.library.parts.first().context("Missing sync book")?.id;
                let position = self
                    .library
                    .progress
                    .iter()
                    .find(|p| p.part_id == part)
                    .map_or(0, |p| p.position);
                self.load(part, position, false)?;
                None
            }
            "rate" => Some(Command::Rate(
                request["value"].as_f64().context("Missing rate")?,
            )),
            "busy_document_import" => {
                let file = self
                    .library
                    .media
                    .first()
                    .context("Missing fixture")?
                    .clone();
                let maintenance = self.maintenance;
                let scan_source = self.scan_source;
                for (moving, scanning) in [(true, None), (false, Some(file.source_id))] {
                    self.maintenance = moving;
                    self.scan_source = scanning;
                    let result = self.handle(Command::OpenDocument(file.clone()));
                    self.maintenance = maintenance;
                    self.scan_source = scan_source;
                    let error = result
                        .err()
                        .context("Document import bypassed source operation guard")?;
                    let expected = if moving {
                        text(
                            "Дождитесь проверки нового расположения источника",
                            "Wait for source relocation to finish",
                        )
                    } else {
                        text(
                            "Дождитесь обновления источника",
                            "Wait for the source update to finish",
                        )
                    };
                    anyhow::ensure!(
                        error.to_string() == expected,
                        "Unexpected import error: {error}"
                    );
                }
                None
            }
            "load_settings_burst" => {
                self.handle(Command::Rate(
                    request["rate"].as_f64().context("Missing rate")?,
                ))?;
                self.handle(Command::SkipSilence(
                    request["silence"].as_bool().context("Missing silence")?,
                ))?;
                Some(Command::Part(number("id") as Id, number("position")))
            }
            "checkpoint" => Some(Command::Hidden(true)),
            "part" => Some(Command::Part(number("id") as Id, number("position"))),
            "resume" => Some(Command::Resume(number("id") as Id)),
            "remove_book" => Some(Command::RemoveBook(number("id") as Id)),
            "silence" => Some(Command::SkipSilence(
                request["value"].as_bool().unwrap_or(false),
            )),
            "rate_step" => Some(Command::RateStep(
                request["value"].as_bool().unwrap_or(false),
            )),
            "mute" => Some(Command::Mute),
            "cancel_scan" => Some(Command::CancelScan),
            "import" => Some(Command::Import),
            op => bail!("Unknown device test operation: {op}"),
        };
        if let Some(command) = command {
            self.handle(command)?;
        }
        let saved = self.store()?.load()?;
        let library = self.library.clone();
        let rates = self.store()?.sync_rates(&library)?;
        Ok(
            json!({"books": self.library.books, "parts": self.library.parts,
            "media": self.library.media, "session": self.library.session,
            "progress": self.library.progress, "saved": saved.session,
            "saved_progress": saved.progress, "settings": self.options.settings,
            "book_rates": rates,
            "sync": self.sync.as_ref().map(|s| s.test_status()),
            "playback": {"position": self.playback.position, "playing": self.playback.playing,
                "phase": format!("{:?}", self.playback.phase), "token": self.token,
                "rate": self.playback.rate, "silence": self.playback.skip_silence,
                "barrier": self.playback.barrier,
                "error": self.playback.error}}),
        )
    }
}
