//! `kclaude.env`: the `KEY=VALUE` file the kclaude scripts source on every run.
//!
//! Values are written unquoted. That is safe because the port is a number, the key is
//! restricted to `[A-Za-z0-9._-]` (see `secret`), the command name to `[A-Za-z0-9_-]`
//! (see `command_name`) and model names are validated against
//! the same kind of character set (see `model`); the tests source the file with sh,
//! bash and zsh to prove it. The PowerShell script parses the same lines itself.

use super::model::AppConfig;

/// Render the file content for `config` and the access `secret`.
///
/// The order is fixed so an unchanged configuration renders to identical bytes and the
/// file does not need rewriting.
pub fn render(config: &AppConfig, secret: &str) -> String {
    format!(
        "KIRO_GATEWAY_PORT={}\n\
         KIRO_GATEWAY_KEY={}\n\
         KCLAUDE_OPUS_MODEL={}\n\
         KCLAUDE_SONNET_MODEL={}\n\
         KCLAUDE_HAIKU_MODEL={}\n\
         KIRO_GATEWAY_UPGRADE_REQUIRED={}\n\
         KIRO_GATEWAY_COMMAND={}\n",
        config.port,
        secret,
        config.models.opus,
        config.models.sonnet,
        config.models.haiku,
        u8::from(config.upgrade_required),
        config.command_name,
    )
}

#[cfg(test)]
mod tests;
