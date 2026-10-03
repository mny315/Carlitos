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
                let library = self.store()?.import(vec![Draft {
                    root_uri: request["root"].as_str().context("Missing root")?.into(),
                    title: request["title"].as_str().context("Missing title")?.into(),
                    author: "Device suite".into(),
                    files,
                    include: true,
                }])?;
                self.reconcile(library);
                None
            }
            "state" => None,
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
        Ok(
            json!({"books": self.library.books, "parts": self.library.parts,
            "media": self.library.media, "session": self.library.session,
            "progress": self.library.progress, "saved": saved.session,
            "saved_progress": saved.progress, "settings": self.options.settings,
            "playback": {"position": self.playback.position, "playing": self.playback.playing,
                "phase": format!("{:?}", self.playback.phase), "token": self.token,
                "rate": self.playback.rate, "silence": self.playback.skip_silence,
                "barrier": self.playback.barrier,
                "error": self.playback.error}}),
        )
    }
}
