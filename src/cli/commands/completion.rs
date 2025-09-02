use crate::cli::Cli;
use anyhow::Result;
use clap::CommandFactory;
use clap_complete::{generate, Shell};
use colored::Colorize;
use std::io;

/// Generate shell completion scripts
pub fn execute(shell: Shell) -> Result<()> {
    println!("{}", "Generating shell completion...".green());
    println!("  • Shell: {:?}", shell);

    let mut cmd = Cli::command();
    let bin_name = "digstore";

    println!("  • Generating completion script for {}", bin_name.cyan());

    generate(shell, &mut cmd, bin_name, &mut io::stdout());

    println!("\n{}", "✓ Completion script generated!".green());
    println!("\n{}", "Installation Instructions:".bold());

    match shell {
        Shell::Bash => {
            println!("  Add the following to your ~/.bashrc or ~/.bash_profile:");
            println!("  {}", "eval \"$(digstore completion bash)\"".cyan());
            println!("\n  Or save to a file and source it:");
            println!(
                "  {}",
                "digstore completion bash > ~/.local/share/bash-completion/completions/digstore"
                    .cyan()
            );
        },
        Shell::Zsh => {
            println!("  Add the following to your ~/.zshrc:");
            println!("  {}", "eval \"$(digstore completion zsh)\"".cyan());
            println!("\n  Or save to a file in your fpath:");
            println!(
                "  {}",
                "digstore completion zsh > ~/.local/share/zsh/site-functions/_digstore".cyan()
            );
        },
        Shell::Fish => {
            println!("  Save the completion script:");
            println!(
                "  {}",
                "digstore completion fish > ~/.config/fish/completions/digstore.fish".cyan()
            );
        },
        Shell::PowerShell => {
            println!("  Add the following to your PowerShell profile:");
            println!(
                "  {}",
                "Invoke-Expression (& digstore completion powershell)".cyan()
            );
            println!("\n  Or save to a file and import it:");
            println!(
                "  {}",
                "digstore completion powershell > $PROFILE\\..\\Modules\\digstore\\digstore.psm1"
                    .cyan()
            );
        },
        Shell::Elvish => {
            println!("  Save the completion script:");
            println!(
                "  {}",
                "digstore completion elvish > ~/.config/elvish/completions/digstore.elv".cyan()
            );
        },
        _ => {
            println!(
                "  Please refer to your shell's documentation for installing completion scripts."
            );
        },
    }

    println!("\n{}", "Features enabled by completion:".bold());
    println!("  • Command and subcommand completion");
    println!("  • Option and flag completion");
    println!("  • File path completion for relevant arguments");
    println!("  • Store ID and hash completion (where applicable)");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap_complete::Shell;

    #[test]
    fn test_completion_generation() {
        // Test that we can generate completion for different shells
        let shells = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell];

        for shell in shells {
            // This would normally write to stdout, but we're just testing the structure
            let mut cmd = Cli::command();
            let bin_name = "digstore";

            // Just verify we can call generate without panicking
            let mut output = Vec::new();
            generate(shell, &mut cmd, bin_name, &mut output);

            // Verify some output was generated
            assert!(
                !output.is_empty(),
                "No completion script generated for {:?}",
                shell
            );
        }
    }
}
