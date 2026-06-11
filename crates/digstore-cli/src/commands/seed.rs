use crate::cli::{SeedAction, SeedArgs};
use crate::error::CliError;
use crate::ops::wallet::resolve_passphrase;
use crate::ui::Ui;
use digstore_chain::{config, seed, unlock};
use zeroize::Zeroizing;

pub fn run(ui: &Ui, args: SeedArgs) -> Result<(), CliError> {
    let home = config::dig_home().map_err(CliError::from)?;
    let seed_path = config::seed_path(&home);
    let session_path = config::session_path(&home);

    match args.action {
        SeedAction::Import { mnemonic } => {
            let phrase = match mnemonic {
                Some(m) => seed::validate_mnemonic(&m).map_err(CliError::from)?,
                None => {
                    let raw = Zeroizing::new(
                        ui.prompt_line("Enter your BIP-39 mnemonic", "")
                            .ok_or_else(|| CliError::InvalidMnemonic("no input".into()))?,
                    );
                    seed::validate_mnemonic(&raw).map_err(CliError::from)?
                }
            };
            let pass = resolve_passphrase(ui, "Set a passphrase to encrypt your seed")?;
            let enc = seed::encrypt_seed(&phrase, &pass).map_err(CliError::from)?;
            seed::save_seed(&seed_path, &enc).map_err(CliError::from)?;
            let cfg = config::GlobalConfig::load(&home).map_err(CliError::from)?;
            unlock::write_session(&session_path, &phrase, cfg.unlock_ttl)
                .map_err(CliError::from)?;
            ui.success("seed imported and unlocked");
            Ok(())
        }
        SeedAction::Generate { words } => {
            let cfg = config::GlobalConfig::load(&home).map_err(CliError::from)?;
            let pass = resolve_passphrase(ui, "Set a passphrase to encrypt your seed")?;
            let phrase = seed::generate_mnemonic(words).map_err(CliError::from)?;
            if !ui.json() {
                ui.line("");
                ui.line("Your new mnemonic — write it down and store it safely:");
                ui.line("");
                ui.line(format!("    {}", &*phrase));
                ui.line("");
            }
            let enc = seed::encrypt_seed(&phrase, &pass).map_err(CliError::from)?;
            seed::save_seed(&seed_path, &enc).map_err(CliError::from)?;
            unlock::write_session(&session_path, &phrase, cfg.unlock_ttl)
                .map_err(CliError::from)?;
            ui.success("seed generated and unlocked");
            Ok(())
        }
        SeedAction::Status => {
            let exists = seed::seed_exists(&seed_path);
            let unlocked = unlock::is_unlocked(&session_path);
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "seed_exists": exists,
                    "unlocked": unlocked,
                }));
            } else if !exists {
                ui.line("no seed (run `digstore seed import` or `digstore seed generate`)");
            } else if unlocked {
                ui.line("seed: present, unlocked");
            } else {
                ui.line("seed: present, locked");
            }
            Ok(())
        }
    }
}
