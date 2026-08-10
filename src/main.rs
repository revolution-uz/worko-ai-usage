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
    time::{Duration, SystemTime},
};
use walkdir::WalkDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Connect this computer to a Worko account
    Login {
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        email: Option<String>,
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
    observed_at: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    sessions: u32,
    five_hour_percent: Option<f64>,
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
        Command::Login { url, email } => login(url, email),
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

fn login(url: Option<String>, email: Option<String>) -> Result<()> {
    let base_url = prompt_or(url, "Worko URL (for example https://hr.example.com): ")?
        .trim_end_matches('/')
        .to_owned();
    let identifier = prompt_or(email, "Email or phone: ")?;
    print!("Password: ");
    io::stdout().flush()?;
    let password = rpassword::read_password()?;
    let response = client().post(format!("{base_url}/api/v1/auth/login"))
        .json(&json!({"identifier": identifier, "password": password, "device_name": "worko-ai-usage"}))
        .send().context("Worko server is unreachable")?;
    if !response.status().is_success() {
        bail!("login failed ({})", response.status());
    }
    let body: Value = response.json()?;
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .context("login response has no token")?;
    save_config(&Config {
        base_url,
        token: token.to_owned(),
    })?;
    println!("Connected. Claude/Codex login credentials are never read or uploaded.");
    sync()
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
    let start = snapshots.len().saturating_sub(48);
    let response = client()
        .post(format!(
            "{}/api/v1/ai-agent-usage/snapshots",
            config.base_url
        ))
        .bearer_auth(config.token)
        .json(&json!({"snapshots": &snapshots[start..]}))
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
    let mut buckets: BTreeMap<(String, String), Snapshot> = BTreeMap::new();

    for (provider, root) in sources {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root)
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
                let key = (provider.to_owned(), hour.clone());
                let bucket = buckets.entry(key).or_insert_with(|| Snapshot {
                    provider: provider.to_owned(),
                    machine_id: machine_id.clone(),
                    observed_at: hour,
                    input_tokens: 0,
                    cached_input_tokens: 0,
                    output_tokens: 0,
                    sessions: 0,
                    five_hour_percent: None,
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
                }
            }
        }
    }
    Ok(buckets.into_values().collect())
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

fn prompt_or(value: Option<String>, label: &str) -> Result<String> {
    if let Some(value) = value {
        return Ok(value);
    }
    print!("{label}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_owned())
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
