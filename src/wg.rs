use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Peer {
    pub public_key: String,
    pub endpoint: Option<String>,
    pub allowed_ips: Vec<String>,
    pub latest_handshake: u64,
    pub receive_bytes: u64,
    pub transmit_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub name: String,
    pub public_key: String,
    pub listen_port: u16,
    pub peers: Vec<Peer>,
}

pub fn devices() -> Result<Vec<Device>, String> {
    let output = Command::new("wg")
        .args(["show", "all", "dump"])
        .output()
        .map_err(|e| format!("wg show all dump: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "wg show all dump: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(parse_dump(&String::from_utf8_lossy(&output.stdout)))
}

pub fn parse_dump(text: &str) -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();

    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();

        match fields.len() {
            5 => devices.push(Device {
                name: fields[0].to_string(),
                public_key: fields[2].to_string(),
                listen_port: fields[3].parse().unwrap_or(0),
                peers: Vec::new(),
            }),
            9 => {
                let peer = Peer {
                    public_key: fields[1].to_string(),
                    endpoint: if fields[3] == "(none)" {
                        None
                    } else {
                        Some(fields[3].to_string())
                    },
                    allowed_ips: fields[4]
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect(),
                    latest_handshake: fields[5].parse().unwrap_or(0),
                    receive_bytes: fields[6].parse().unwrap_or(0),
                    transmit_bytes: fields[7].parse().unwrap_or(0),
                };

                if let Some(device) = devices.iter_mut().find(|d| d.name == fields[0]) {
                    device.peers.push(peer);
                }
            }
            _ => {}
        }
    }

    devices
}

pub fn wg_quick(action: &str, name: &str) -> Result<(), String> {
    let output = Command::new("wg-quick")
        .args([action, name])
        .output()
        .map_err(|e| format!("{}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "exit status {}\nwg-quick's output:\n{}",
            output.status.code().unwrap_or(-1),
            stderr.trim_end()
        ));
    }

    Ok(())
}

pub fn set_peer_remove(iface: &str, public_key: &str) -> Result<(), String> {
    let output = Command::new("wg")
        .args(["set", iface, "peer", public_key, "remove"])
        .output()
        .map_err(|e| format!("{}", e))?;

    if !output.status.success() {
        return Err(format!(
            "wg set {} peer ... remove: {} (exit status {})",
            iface,
            String::from_utf8_lossy(&output.stderr).trim(),
            output.status.code().unwrap_or(-1)
        ));
    }

    Ok(())
}

pub fn addconf(iface: &str, path: &str) -> Result<(), String> {
    let output = Command::new("wg")
        .args(["addconf", iface, path])
        .output()
        .map_err(|e| format!("{}", e))?;

    if !output.status.success() {
        return Err(format!(
            "wg addconf {}: {} (exit status {})",
            iface,
            String::from_utf8_lossy(&output.stderr).trim(),
            output.status.code().unwrap_or(-1)
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wg_dump() {
        let dump = "wg0\taaa\tbbb\t51820\toff\n\
                    wg0\tccc\t(none)\t1.2.3.4:51820\t10.0.0.2/32\t1700000000\t1234\t5678\t25\n";
        let devices = parse_dump(dump);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "wg0");
        assert_eq!(devices[0].public_key, "bbb");
        assert_eq!(devices[0].listen_port, 51820);
        assert_eq!(devices[0].peers.len(), 1);
        assert_eq!(devices[0].peers[0].public_key, "ccc");
        assert_eq!(
            devices[0].peers[0].endpoint.as_deref(),
            Some("1.2.3.4:51820")
        );
        assert_eq!(devices[0].peers[0].receive_bytes, 1234);
        assert_eq!(devices[0].peers[0].transmit_bytes, 5678);
    }

    #[test]
    fn parses_empty_dump() {
        assert!(parse_dump("").is_empty());
    }
}
