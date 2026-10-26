use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::conf::Conf;
use crate::endpoints::{self, EndpointKind};
use crate::humanize;
use crate::settings::Settings;
use crate::wg;

pub const TUNNELS_PATH: &str = "/etc/wireguard/";

// set once the remembered servers of the previous run have been reconnected;
// before that, disconnecting a tunnel must not wipe the remembered list
pub static RECONNECTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
pub struct PeerDisplay {
    pub comment: String,
    pub public_key: String,
    pub allowed_ips: String,
    pub endpoint: String,
    pub handshake: String,
    pub transfer: String,
    pub active: bool,
}

pub fn tunnel_path(name: &str) -> String {
    format!("{}{}.conf", TUNNELS_PATH, name)
}

pub fn active_device_names() -> Vec<String> {
    wg::devices()
        .map(|devices| devices.into_iter().map(|d| d.name).collect())
        .unwrap_or_default()
}

pub fn peer_transfer_string(peer: &wg::Peer) -> String {
    format!(
        "{} received, {} sent",
        humanize::bytes(peer.receive_bytes),
        humanize::bytes(peer.transmit_bytes)
    )
}

// builds the peer list of a tunnel by merging its config file (every [Peer]
// section) with the live data of its wireguard device
pub fn peer_displays(conf: &Conf, device: Option<&wg::Device>) -> Vec<PeerDisplay> {
    let mut live: HashMap<&str, &wg::Peer> = HashMap::new();
    if let Some(device) = device {
        for peer in &device.peers {
            live.insert(peer.public_key.as_str(), peer);
        }
    }

    let mut peers: Vec<PeerDisplay> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for section in conf.peers() {
        let mut display = PeerDisplay {
            comment: section.comment.trim_start_matches('#').trim().to_string(),
            public_key: section.get("PublicKey").unwrap_or("").trim().to_string(),
            allowed_ips: section.get("AllowedIPs").unwrap_or("").trim().to_string(),
            endpoint: section.get("Endpoint").unwrap_or("").trim().to_string(),
            handshake: "unknown".to_string(),
            transfer: "unknown".to_string(),
            active: false,
        };

        seen.push(display.public_key.clone());

        if let Some(peer) = live.get(display.public_key.as_str()) {
            display.active = true;
            if let Some(endpoint) = &peer.endpoint {
                display.endpoint = endpoint.clone();
            }
            display.handshake = humanize::time(peer.latest_handshake);
            display.transfer = peer_transfer_string(peer);
        }

        peers.push(display);
    }

    // include peers that are set on the running device but not in the config
    if let Some(device) = device {
        for peer in &device.peers {
            if seen.iter().any(|k| k == &peer.public_key) {
                continue;
            }

            let mut display = PeerDisplay {
                comment: String::new(),
                public_key: peer.public_key.clone(),
                allowed_ips: peer.allowed_ips.join(", "),
                endpoint: String::new(),
                handshake: humanize::time(peer.latest_handshake),
                transfer: peer_transfer_string(peer),
                active: true,
            };

            if let Some(endpoint) = &peer.endpoint {
                display.endpoint = endpoint.clone();
            }

            seen.push(peer.public_key.clone());
            peers.push(display);
        }
    }

    peers
}

pub fn toggle_tunnel(name: &str, settings: &Mutex<Settings>) -> Result<(), String> {
    let devices = wg::devices()?;
    let active_names: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();

    let disconnect = |tunnel: &str| -> Result<(), String> {
        wg::wg_quick("down", tunnel)?;

        let mut current = settings.lock().unwrap();
        if current.remember_connected_servers
            && !current.connected_servers.is_empty()
            && RECONNECTED.load(Ordering::SeqCst)
        {
            current.connected_servers.remove(tunnel);
            if let Err(e) = current.save() {
                eprintln!("Error saving settings on disconnect: {}", e);
            }
        }

        Ok(())
    };

    let multiple_tunnels = settings.lock().unwrap().multiple_tunnels;

    if multiple_tunnels {
        if devices.iter().any(|d| d.name == name) {
            disconnect(name)?;
        }
    } else {
        for device in &devices {
            disconnect(&device.name)?;
        }
    }

    // dont connect to the new one as this is a disconnect action
    if active_names.contains(&name.to_string()) {
        return Ok(());
    }

    wg::wg_quick("up", name)?;

    let mut current = settings.lock().unwrap();
    if current.remember_connected_servers {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        current.connected_servers.insert(name.to_string(), now);
        if let Err(e) = current.save() {
            eprintln!("Error saving settings on connect: {}", e);
        }
    }

    Ok(())
}

fn peer_endpoint_key(tunnel: &str, public_key: &str) -> String {
    format!("{}|{}", tunnel, public_key)
}

// stores the endpoint a peer is currently using, because removing a peer makes
// the kernel forget it
pub fn remember_peer_endpoint(
    tunnel: &str,
    public_key: &str,
    peer_endpoints: &Mutex<HashMap<String, String>>,
) {
    let devices = match wg::devices() {
        Ok(devices) => devices,
        Err(_) => return,
    };

    for device in devices.iter().filter(|d| d.name == tunnel) {
        for peer in &device.peers {
            if peer.public_key != public_key {
                continue;
            }

            if let Some(endpoint) = &peer.endpoint {
                peer_endpoints
                    .lock()
                    .unwrap()
                    .insert(peer_endpoint_key(tunnel, public_key), endpoint.clone());
            }

            return;
        }
    }
}

// re-adds a single peer to the running interface using the values of the
// config file; wg addconf is additive, so other peers are untouched
pub fn activate_peer(
    tunnel: &str,
    public_key: &str,
    peer_endpoints: &Mutex<HashMap<String, String>>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(tunnel_path(tunnel)).map_err(|e| e.to_string())?;
    let conf = Conf::parse(&text);

    let peer = conf
        .peers()
        .into_iter()
        .find(|section| section.get("PublicKey").map(|v| v.trim()) == Some(public_key))
        .ok_or_else(|| format!("peer {} is not defined in {}.conf", public_key, tunnel))?;

    // use the config's endpoint, or the one the peer had before being removed
    let mut endpoint = peer.get("Endpoint").unwrap_or("").trim().to_string();
    if endpoint.is_empty() {
        endpoint = peer_endpoints
            .lock()
            .unwrap()
            .get(&peer_endpoint_key(tunnel, public_key))
            .cloned()
            .unwrap_or_default();
    }

    let mut body = String::from("[Peer]\n");
    for key in ["PublicKey", "PresharedKey", "AllowedIPs", "PersistentKeepalive"] {
        if let Some(value) = peer.get(key) {
            let value = value.trim();
            if !value.is_empty() {
                body.push_str(&format!("{} = {}\n", key, value));
            }
        }
    }
    if !endpoint.is_empty() {
        body.push_str(&format!("Endpoint = {}\n", endpoint));
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = std::env::temp_dir().join(format!(
        "wg-gui-peer-{}-{}.conf",
        std::process::id(),
        unique
    ));

    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .map_err(|e| e.to_string())?;

        file.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }

    let result = wg::addconf(tunnel, &tmp_path.to_string_lossy());
    let _ = std::fs::remove_file(&tmp_path);
    result?;

    // with a known endpoint the server can initiate the handshake itself, so
    // send one packet to make the kernel do it right away
    if !endpoint.is_empty() {
        for cidr in peer.get("AllowedIPs").unwrap_or("").split(',') {
            let cidr = cidr.trim();
            let ip = match cidr
                .split('/')
                .next()
                .and_then(|s| s.parse::<IpAddr>().ok())
            {
                Some(ip) => ip,
                None => continue,
            };

            let ip_string = ip.to_string();
            let _ = Command::new("ping")
                .args(["-c", "1", "-W", "1", ip_string.as_str()])
                .status();
            break;
        }
    }

    Ok(())
}

// removes a single peer from the running interface, keeping it in the config
pub fn deactivate_peer(tunnel: &str, public_key: &str) -> Result<(), String> {
    wg::set_peer_remove(tunnel, public_key)
}

// resolves the endpoint of a tunnel's first peer to an IP, used for the
// country flags
pub fn endpoint_ip(conf: &Conf) -> Result<String, String> {
    let endpoint = conf
        .section("Peer")
        .and_then(|section| section.get("Endpoint"))
        .unwrap_or("");

    match endpoints::categorize(endpoint) {
        EndpointKind::Ipv4 | EndpointKind::Ipv6 => Ok(endpoints::address(endpoint).to_string()),
        EndpointKind::Fqdn => {
            let host = endpoints::address(endpoint);
            let addrs = (host, 0)
                .to_socket_addrs()
                .map_err(|e| format!("dns lookup error: {}", e))?;

            addrs
                .into_iter()
                .next()
                .map(|addr| addr.ip().to_string())
                .ok_or_else(|| format!("endpoint \"{}\" coudln't find ip from dns", host))
        }
        EndpointKind::Unknown => Err(format!("endpoint \"{}\" is invalid", endpoint)),
    }
}

pub fn parse_zip_archive(target: &str) -> Result<(), String> {
    let file = std::fs::File::open(target).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }

        let name = entry.name().to_string();
        let base = name.rsplit('/').next().unwrap_or(&name).to_string();

        // parse only .conf files from zip archive
        if !base.ends_with(".conf") {
            continue;
        }

        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
        std::fs::write(format!("{}{}", TUNNELS_PATH, base), data).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn zip_options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o777)
}

fn add_tree_to_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &Path,
    prefix: &str,
) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let relative = format!("{}/{}", prefix, name);

        if path.is_dir() {
            zip.add_directory(format!("{}/", relative), zip_options())
                .map_err(|e| e.to_string())?;
            add_tree_to_zip(zip, &path, &relative)?;
        } else {
            zip.start_file(relative, zip_options())
                .map_err(|e| e.to_string())?;

            let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, zip).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

pub fn zip_tunnels(dest: &str) -> Result<(), String> {
    let file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);

    let base = Path::new(dest)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    zip.add_directory(format!("{}/", base), zip_options())
        .map_err(|e| e.to_string())?;
    add_tree_to_zip(&mut zip, Path::new(TUNNELS_PATH), &base)?;
    zip.finish().map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_ip_handles_ipv6_endpoint_with_port() {
        let conf = Conf::parse("[Peer]\nEndpoint = [2001:db8::1]:51820\n");
        assert_eq!(endpoint_ip(&conf).unwrap(), "2001:db8::1");
    }

    #[test]
    fn endpoint_ip_handles_ipv4_endpoint_with_port() {
        let conf = Conf::parse("[Peer]\nEndpoint = 192.0.2.1:51820\n");
        assert_eq!(endpoint_ip(&conf).unwrap(), "192.0.2.1");
    }

    #[test]
    fn merges_config_and_live_peers() {
        let conf = Conf::parse(
            "# note\n[Peer]\nPublicKey = k1\nAllowedIPs = 10.0.0.0/24\n[Peer]\nPublicKey = k2\n",
        );

        let device = wg::Device {
            name: "wg0".to_string(),
            public_key: "ifacekey".to_string(),
            listen_port: 51820,
            peers: vec![
                wg::Peer {
                    public_key: "k1".to_string(),
                    endpoint: Some("1.2.3.4:51820".to_string()),
                    allowed_ips: vec!["10.0.0.0/24".to_string()],
                    latest_handshake: 0,
                    receive_bytes: 0,
                    transmit_bytes: 0,
                },
                wg::Peer {
                    public_key: "k3".to_string(),
                    endpoint: None,
                    allowed_ips: vec!["10.9.0.0/16".to_string()],
                    latest_handshake: 0,
                    receive_bytes: 0,
                    transmit_bytes: 0,
                },
            ],
        };

        let peers = peer_displays(&conf, Some(&device));

        assert_eq!(peers.len(), 3);
        assert_eq!(peers[0].public_key, "k1");
        assert_eq!(peers[0].comment, "note");
        assert!(peers[0].active);
        assert_eq!(peers[0].endpoint, "1.2.3.4:51820");
        assert!(!peers[1].active);
        assert_eq!(peers[1].public_key, "k2");
        assert_eq!(peers[2].public_key, "k3");
        assert_eq!(peers[2].allowed_ips, "10.9.0.0/16");
        assert!(peers[2].active);
    }

    #[test]
    fn no_device_means_inactive_peers() {
        let conf = Conf::parse("[Peer]\nPublicKey = k1\nEndpoint = 1.2.3.4:51820\n");
        let peers = peer_displays(&conf, None);

        assert_eq!(peers.len(), 1);
        assert!(!peers[0].active);
        assert_eq!(peers[0].endpoint, "1.2.3.4:51820");
        assert_eq!(peers[0].handshake, "unknown");
        assert_eq!(peers[0].transfer, "unknown");
    }
}
