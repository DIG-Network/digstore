//! CLI context for storing global options during command execution

use std::cell::RefCell;

thread_local! {
    static CLI_CONTEXT: RefCell<Option<CliContext>> = const { RefCell::new(None) };
}

/// Context containing global CLI options
#[derive(Debug, Clone, Default)]
pub struct CliContext {
    pub wallet_profile: Option<String>,
    pub auto_generate_wallet: bool,
    pub auto_import_mnemonic: Option<String>,
    pub verbose: bool,
    pub quiet: bool,
    pub yes: bool,
    pub non_interactive: bool,
    pub custom_encryption_key: Option<String>,
    pub custom_decryption_key: Option<String>,
}

impl CliContext {
    /// Set the global CLI context for the current thread
    pub fn set(context: CliContext) {
        CLI_CONTEXT.with(|c| {
            *c.borrow_mut() = Some(context);
        });
    }

    /// Get the current CLI context
    pub fn get() -> Option<CliContext> {
        CLI_CONTEXT.with(|c| c.borrow().clone())
    }

    /// Get the wallet profile from the current context
    pub fn get_wallet_profile() -> Option<String> {
        Self::get().and_then(|ctx| ctx.wallet_profile)
    }

    /// Check if verbose mode is enabled
    pub fn is_verbose() -> bool {
        Self::get().map(|ctx| ctx.verbose).unwrap_or(false)
    }

    /// Check if quiet mode is enabled
    pub fn is_quiet() -> bool {
        Self::get().map(|ctx| ctx.quiet).unwrap_or(false)
    }

    /// Check if auto-answer yes is enabled
    pub fn is_yes() -> bool {
        Self::get().map(|ctx| ctx.yes).unwrap_or(false)
    }

    /// Check if non-interactive mode is enabled
    pub fn is_non_interactive() -> bool {
        Self::get().map(|ctx| ctx.non_interactive).unwrap_or(false)
    }

    /// Check if we should auto-accept prompts (yes flag OR non-interactive mode)
    pub fn should_auto_accept() -> bool {
        Self::is_yes() || Self::is_non_interactive()
    }

    /// Get custom encryption key from the current context
    pub fn get_custom_encryption_key() -> Option<String> {
        Self::get().and_then(|ctx| ctx.custom_encryption_key)
    }

    /// Get custom decryption key from the current context
    pub fn get_custom_decryption_key() -> Option<String> {
        Self::get().and_then(|ctx| ctx.custom_decryption_key)
    }
}
