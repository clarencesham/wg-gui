use std::collections::HashMap;
use std::io::Read;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

const MMDB_URL: &str = "https://github.com/iplocate/ip-address-databases/raw/refs/heads/main/ip-to-country/ip-to-country.mmdb";
const MMDB_FILE: &str = "./countries.mmdb";

static DB: OnceLock<Result<maxminddb::Reader<Vec<u8>>, String>> = OnceLock::new();
static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

#[derive(Deserialize)]
struct IpRecord {
    country_code: Option<String>,
    country: Option<CountryRecord>,
}

#[derive(Deserialize)]
struct CountryRecord {
    iso_code: Option<String>,
}

fn cache() -> &'static Mutex<HashMap<String, String>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn database() -> &'static Result<maxminddb::Reader<Vec<u8>>, String> {
    DB.get_or_init(init_database)
}

pub fn open_database() -> Result<(), String> {
    database().as_ref().map(|_| ()).map_err(|e| e.clone())
}

pub fn find(ip_str: &str) -> Result<String, String> {
    let reader = database()
        .as_ref()
        .map_err(|e| format!("database initialization failed: {}", e))?;

    if let Some(value) = cache().lock().unwrap().get(ip_str) {
        return Ok(value.clone());
    }

    let ip: IpAddr = ip_str
        .parse()
        .map_err(|_| format!("invalid IP address format: {}", ip_str))?;

    let result = reader
        .lookup(ip)
        .map_err(|e| format!("error during database lookup: {}", e))?;

    let record: Option<IpRecord> = if result.has_data() {
        result
            .decode()
            .map_err(|e| format!("error during database lookup: {}", e))?
    } else {
        None
    };

    if let Some(record) = record {
        let code = record
            .country_code
            .or_else(|| record.country.and_then(|c| c.iso_code));

        if let Some(code) = code {
            cache().lock().unwrap().insert(ip_str.to_string(), code.clone());
            return Ok(code);
        }
    }

    Ok("Unknown".to_string())
}

fn init_database() -> Result<maxminddb::Reader<Vec<u8>>, String> {
    let needs_download = match std::fs::metadata(MMDB_FILE) {
        Ok(metadata) => metadata
            .modified()
            .ok()
            .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
            .map(|age| age > Duration::from_secs(7 * 24 * 60 * 60))
            .unwrap_or(false),
        Err(_) => true,
    };

    if needs_download {
        download_database().map_err(|e| format!("failed to download country database: {}", e))?;
    }

    maxminddb::Reader::open_readfile(MMDB_FILE)
        .map_err(|e| format!("failed to open country database: {}", e))
}

fn download_database() -> Result<(), String> {
    let response = ureq::get(MMDB_URL)
        .set("User-Agent", "wg-gui")
        .call()
        .map_err(|e| format!("failed to download mmdb: {}", e))?;

    if response.status() != 200 {
        return Err(format!("unexpected status code: {}", response.status()));
    }

    let mut reader = response.into_reader();
    let mut data = Vec::new();
    reader
        .read_to_end(&mut data)
        .map_err(|e| format!("failed to download mmdb: {}", e))?;

    std::fs::write(MMDB_FILE, data).map_err(|e| format!("failed to write data to disk: {}", e))
}
