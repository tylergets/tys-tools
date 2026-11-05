//! Spots secrets (API keys, tokens, private keys, credentials in URLs) in copied text so
//! they are never written to history.

use anyhow::{Context, Result};
use regex::RegexSet;

use crate::config::SecretsConfig;

/// Built-in patterns. Each one targets a token with a distinctive shape, so ordinary text
/// and code rarely match.
const BUILTIN_PATTERNS: &[&str] = &[
    // PEM private keys (RSA, EC, OpenSSH, PKCS#8, encrypted).
    r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----",
    // AWS access key ids.
    r"\b(AKIA|ASIA)[0-9A-Z]{16}\b",
    // GitHub tokens.
    r"\bgh[pousr]_[A-Za-z0-9]{36,}\b",
    r"\bgithub_pat_[A-Za-z0-9_]{22,}\b",
    // GitLab personal access tokens.
    r"\bglpat-[A-Za-z0-9_-]{20,}",
    // Slack tokens.
    r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
    // OpenAI / Anthropic style keys.
    r"\bsk-(ant-|proj-)?[A-Za-z0-9_-]{20,}",
    // Stripe secret and restricted keys.
    r"\b[sr]k_(live|test)_[A-Za-z0-9]{16,}",
    // Google API keys.
    r"\bAIza[0-9A-Za-z_-]{35}\b",
    // npm tokens.
    r"\bnpm_[A-Za-z0-9]{36}\b",
    // JSON Web Tokens.
    r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    // Credentials embedded in a URL, e.g. postgres://user:pass@host.
    r"\b[a-zA-Z][a-zA-Z0-9+.-]*://[^/\s:@]+:[^/\s@]+@",
];

#[derive(Debug, Clone, Default)]
pub struct SecretFilter {
    patterns: Option<RegexSet>,
}

impl SecretFilter {
    pub fn from_config(config: &SecretsConfig) -> Result<Self> {
        if !config.enabled {
            return Ok(Self::default());
        }
        let builtin: &[&str] = if config.builtin_patterns {
            BUILTIN_PATTERNS
        } else {
            &[]
        };
        let builtin = builtin.iter().copied();
        let extra = config.patterns.iter().map(String::as_str);
        let patterns = RegexSet::new(builtin.chain(extra))
            .context("invalid regex in clipboard.secrets.patterns")?;
        Ok(Self {
            patterns: (!patterns.is_empty()).then_some(patterns),
        })
    }

    pub fn is_secret(&self, text: &str) -> bool {
        self.patterns.as_ref().is_some_and(|set| set.is_match(text))
    }
}

#[cfg(test)]
mod tests {
    use super::SecretFilter;
    use crate::config::SecretsConfig;

    fn default_filter() -> SecretFilter {
        SecretFilter::from_config(&SecretsConfig::default()).unwrap()
    }

    #[test]
    fn detects_builtin_secrets() {
        let filter = default_filter();
        let secrets = [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk=\n",
            "-----BEGIN RSA PRIVATE KEY-----",
            "aws_access_key_id = AKIAIOSFODNN7EXAMPLE",
            "ghp_0123456789abcdefghijABCDEFGHIJ012345",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "glpat-abcdefghij0123456789",
            "xoxb-1234567890-abcdefghij",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            "sk-proj-abcdefghijklmnopqrstuvwxyz",
            "sk_live_abcdefghijklmnop1234",
            "AIzaSyA-abcdefghijklmnopqrstuvwxyz01234",
            "npm_abcdefghijklmnopqrstuvwxyz0123456789",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "postgres://admin:hunter2@db.internal:5432/app",
        ];
        for secret in secrets {
            assert!(filter.is_secret(secret), "should detect: {secret}");
        }
    }

    #[test]
    fn ignores_ordinary_text() {
        let filter = default_filter();
        let ordinary = [
            "hello world",
            "https://github.com/tylergets/tys-tools",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample user@host",
            "-----BEGIN PUBLIC KEY-----",
            "let password = read_password();",
            "git@github.com:tylergets/tys-tools.git",
            "the task-management sk-ills list",
        ];
        for text in ordinary {
            assert!(!filter.is_secret(text), "false positive: {text}");
        }
    }

    #[test]
    fn custom_patterns_and_disable() {
        let config = SecretsConfig {
            builtin_patterns: false,
            patterns: vec![r"^hunter\d$".to_string()],
            ..SecretsConfig::default()
        };
        let filter = SecretFilter::from_config(&config).unwrap();
        assert!(filter.is_secret("hunter2"));
        assert!(!filter.is_secret("AKIAIOSFODNN7EXAMPLE"));

        let disabled = SecretsConfig {
            enabled: false,
            ..SecretsConfig::default()
        };
        let filter = SecretFilter::from_config(&disabled).unwrap();
        assert!(!filter.is_secret("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn rejects_invalid_pattern() {
        let config = SecretsConfig {
            patterns: vec!["(".to_string()],
            ..SecretsConfig::default()
        };
        assert!(SecretFilter::from_config(&config).is_err());
    }
}
