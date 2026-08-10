use anyhow::{bail, Context, Result};
use chrono::{DateTime, Timelike, Utc};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::{Duration, SystemTime},
};
use walkdir::WalkDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_BASE_URL: &str = "https://hr-platform.uz";

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Connect this computer to a Worko account
    #[command(alias = "connect")]
    Login {
        #[arg(long, default_value = DEFAULT_BASE_URL)]
        url: Option<String>,
        /// One-time token generated in Worko HR (omit to enter it securely)
        #[arg(long, env = "WORKO_AI_USAGE_ONE_TIME_TOKEN", hide_env_values = true)]
        token: Option<String>,
    },
    /// Scan local agent logs and upload hourly counters
    Sync,
    /// Print locally detected hourly counters without uploading
    Status,
    /// Remove the locally stored Worko access token
    Logout,
}

#[derive(Serialize, Deserialize)]
struct Config {
    base_url: String,
    token: String,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    provider: String,
    machine_id: String,
    session_hash: String,
    observed_at: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    sessions: u32,
    five_hour_percent: Option<f64>,
    limit_source: Option<String>,
    collector_version: String,
    ide: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Login { url, token } => login(url, token),
        Command::Sync => sync(),
        Command::Status => {
            println!("{}", serde_json::to_string_pretty(&collect()?)?);
            Ok(())
        }
        Command::Logout => {
            let path = config_path()?;
            if path.exists() {
                fs::remove_file(path)?;
            }
            println!("Logged out. AI provider credentials were not changed.");
            Ok(())
        }
    }
}

fn login(url: Option<String>, token: Option<String>) -> Result<()> {
    let base_url = normalize_base_url(url.as_deref().unwrap_or(DEFAULT_BASE_URL))?;
    let one_time_token = match token {
        Some(token) => token,
        None => {
            let profile_url = format!("{base_url}/profile/integrations/ai-usage");
            println!("Opening Worko HR to generate a one-time token:");
            println!("{profile_url}");
            if !open_in_browser(&profile_url) {
                println!("Could not open a browser automatically. Open the link above manually.");
            }
            print!("One-time token: ");
            io::stdout().flush()?;
            rpassword::read_password()?
        }
    };
    if one_time_token.trim().is_empty() {
        bail!("one-time token cannot be empty");
    }

    let response = client()
        .post(format!("{base_url}/api/v1/ai-agent-usage/auth/exchange"))
        .json(&json!({
            "one_time_token": one_time_token,
            "device_name": hostname::get()?.to_string_lossy(),
            "collector_version": VERSION,
        }))
        .send()
        .context("Worko server is unreachable")?;
    let status = response.status();
    if !status.is_success() {
        let message = response.json::<Value>().ok().and_then(|body| {
            body.pointer("/message")
                .or_else(|| body.pointer("/error/message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if let Some(message) = message {
            bail!("one-time token exchange failed ({status}): {message}");
        }
        bail!("one-time token exchange failed ({status})");
    }
    let body: Value = response.json()?;
    let access_token = body
        .pointer("/token")
        .or_else(|| body.pointer("/access_token"))
        .or_else(|| body.pointer("/data/token"))
        .or_else(|| body.pointer("/data/access_token"))
        .and_then(Value::as_str)
        .context("token exchange response has no access token")?;
    save_config(&Config {
        base_url,
        token: access_token.to_owned(),
    })?;
    println!("Connected. The one-time token has been exchanged and was not stored.");
    sync()
}

fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let result = ProcessCommand::new("open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let result = ProcessCommand::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .spawn();

    #[cfg(all(unix, not(target_os = "macos")))]
    let result = ProcessCommand::new("xdg-open").arg(url).spawn();

    #[cfg(not(any(unix, target_os = "windows")))]
    let result: io::Result<std::process::Child> = Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "browser opening is not supported on this platform",
    ));

    result.is_ok()
}

fn normalize_base_url(value: &str) -> Result<String> {
    let url = reqwest::Url::parse(value).context("Worko URL must be an absolute HTTPS URL")?;
    if url.scheme() != "https" || url.host_str().is_none() {
        bail!("Worko URL must be an absolute HTTPS URL");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("Worko URL cannot contain a query or fragment");
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn sync() -> Result<()> {
    let config: Config = serde_json::from_slice(
        &fs::read(config_path()?).context("not logged in; run `worko-ai-usage login`")?,
    )?;
    let snapshots = collect()?;
    if snapshots.is_empty() {
        println!("No Claude Code or Codex usage events found.");
        return Ok(());
    }
    let recent_cutoff = Utc::now() - chrono::Duration::hours(48);
    let recent: Vec<&Snapshot> = snapshots
        .iter()
        .filter(|snapshot| {
            DateTime::parse_from_rfc3339(&snapshot.observed_at)
                .map(|time| time >= recent_cutoff)
                .unwrap_or(false)
        })
        .rev()
        .take(500)
        .collect();
    let response = client()
        .post(format!(
            "{}/api/v1/ai-agent-usage/snapshots",
            config.base_url
        ))
        .bearer_auth(config.token)
        .json(&json!({"snapshots": recent.into_iter().rev().collect::<Vec<_>>()}))
        .send()?;
    if !response.status().is_success() {
        bail!("sync failed ({})", response.status());
    }
    let body: Value = response.json()?;
    println!(
        "Synced {} hourly snapshots.",
        body.pointer("/data/accepted")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    Ok(())
}

fn collect() -> Result<Vec<Snapshot>> {
    let home = dirs::home_dir().context("home directory not found")?;
    let sources = [
        ("codex", home.join(".codex/sessions")),
        ("claude", home.join(".claude/projects")),
    ];
    let machine_id = machine_id(&home);
    let cutoff = SystemTime::now() - Duration::from_secs(7 * 86_400);
    let mut buckets: BTreeMap<(String, String, String), Snapshot> = BTreeMap::new();

    for (provider, root) in sources {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(|v| v.to_str()) != Some("jsonl")
            {
                continue;
            }
            if fs::metadata(path)?.modified()? < cutoff {
                continue;
            }
            let fallback = DateTime::<Utc>::from(fs::metadata(path)?.modified()?);
            let session_hash = session_hash(&machine_id, provider, path, &root);
            for line in io::BufReader::new(fs::File::open(path)?)
                .lines()
                .map_while(|line| line.ok())
            {
                let Ok(event) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let Some(usage) = find_usage(&event) else {
                    continue;
                };
                let timestamp = find_timestamp(&event).unwrap_or(fallback);
                if timestamp < DateTime::<Utc>::from(cutoff) {
                    continue;
                }
                let hour = timestamp
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap()
                    .to_rfc3339();
                let key = (provider.to_owned(), session_hash.clone(), hour.clone());
                let bucket = buckets.entry(key).or_insert_with(|| Snapshot {
                    provider: provider.to_owned(),
                    machine_id: machine_id.clone(),
                    session_hash: session_hash.clone(),
                    observed_at: hour,
                    input_tokens: 0,
                    cached_input_tokens: 0,
                    output_tokens: 0,
                    sessions: 0,
                    five_hour_percent: None,
                    limit_source: None,
                    collector_version: VERSION.to_owned(),
                    ide: std::env::var("WORKO_IDE").unwrap_or_else(|_| "terminal".to_owned()),
                });
                bucket.input_tokens += number(usage, &["input_tokens", "inputTokens"]);
                bucket.cached_input_tokens +=
                    number(usage, &["cached_input_tokens", "cache_read_input_tokens"]);
                bucket.output_tokens += number(usage, &["output_tokens", "outputTokens"]);
                bucket.sessions += 1;
                if let Some(percent) = find_percent(&event) {
                    bucket.five_hour_percent = Some(percent.clamp(0.0, 100.0));
                    bucket.limit_source = Some("provider_log".to_owned());
                }
            }
        }
    }
    Ok(buckets.into_values().collect())
}

fn session_hash(machine_id: &str, provider: &str, path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut hash = Sha256::new();
    hash.update(machine_id.as_bytes());
    hash.update(provider.as_bytes());
    hash.update(relative.to_string_lossy().as_bytes());
    hex::encode(hash.finalize())
}

fn find_usage(value: &Value) -> Option<&Value> {
    if let Value::Object(map) = value {
        for key in ["usage", "token_usage", "last_token_usage"] {
            if let Some(candidate) = map.get(key) {
                if candidate.get("input_tokens").is_some()
                    || candidate.get("output_tokens").is_some()
                {
                    return Some(candidate);
                }
            }
        }
        for child in map.values() {
            if let Some(found) = find_usage(child) {
                return Some(found);
            }
        }
    } else if let Value::Array(items) = value {
        for child in items {
            if let Some(found) = find_usage(child) {
                return Some(found);
            }
        }
    }
    None
}

fn find_percent(value: &Value) -> Option<f64> {
    if let Value::Object(map) = value {
        for key in ["five_hour_percent", "used_percent", "utilization"] {
            if let Some(number) = map.get(key).and_then(Value::as_f64) {
                return Some(number);
            }
        }
        for child in map.values() {
            if let Some(found) = find_percent(child) {
                return Some(found);
            }
        }
    } else if let Value::Array(items) = value {
        for child in items {
            if let Some(found) = find_percent(child) {
                return Some(found);
            }
        }
    }
    None
}

fn find_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    for key in ["timestamp", "created_at", "createdAt"] {
        if let Some(raw) = value.get(key).and_then(Value::as_str) {
            if let Ok(time) = DateTime::parse_from_rfc3339(raw) {
                return Some(time.with_timezone(&Utc));
            }
        }
    }
    None
}

fn number(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn machine_id(home: &Path) -> String {
    let host = hostname::get().unwrap_or_default();
    let mut hash = Sha256::new();
    hash.update(host.to_string_lossy().as_bytes());
    hash.update(std::env::consts::ARCH.as_bytes());
    hash.update(home.to_string_lossy().as_bytes());
    hex::encode(hash.finalize())
}

fn config_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("config directory not found")?
        .join("worko-ai-usage/config.json"))
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, serde_json::to_vec_pretty(config)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("worko-ai-usage/{VERSION}"))
        .build()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_is_an_alias_for_login() {
        let cli =
            Cli::try_parse_from(["worko-ai-usage", "connect", "--token", "wot_test"]).unwrap();

        assert!(matches!(cli.command, Command::Login { .. }));
    }

    #[test]
    fn accepts_and_normalizes_https_base_url() {
        assert_eq!(
            normalize_base_url("https://hr-platform.uz/").unwrap(),
            "https://hr-platform.uz"
        );
    }

    #[test]
    fn rejects_insecure_or_relative_base_url() {
        assert!(normalize_base_url("http://hr-platform.uz").is_err());
        assert!(normalize_base_url("hr-platform.uz").is_err());
    }

    #[test]
    fn finds_codex_last_token_usage() {
        let event =
            json!({"payload":{"info":{"last_token_usage":{"input_tokens":12,"output_tokens":3}}}});
        let usage = find_usage(&event).unwrap();
        assert_eq!(number(usage, &["input_tokens"]), 12);
        assert_eq!(number(usage, &["output_tokens"]), 3);
    }

    #[test]
    fn clamps_provider_percentage_at_upload_boundary() {
        let event = json!({"rate_limit":{"primary":{"used_percent":42.5}}});
        assert_eq!(find_percent(&event), Some(42.5));
    }
}
